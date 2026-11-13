//! Enhanced streaming: a Yandex MP3 is decoded while it downloads, each
//! piece goes through the exciter at once and out as 16-bit WAV over a
//! local HTTP address the player opens, so playback starts within a second
//! or two instead of after the whole track. When the download is done, the
//! full enhancement (with the loudness lift) is cached as FLAC for later plays.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub use symphonia::core::io::MediaSource;

use crate::enhance::{Exciter, Strength};
use crate::yandex::{self, Progress};

/// Bytes that keep arriving, readable from any position while they do.
#[derive(Default)]
pub struct Growing {
    state: Mutex<(Vec<u8>, bool)>,
    more: Condvar,
}

impl Growing {
    pub fn push(&self, bytes: &[u8]) {
        self.lock().0.extend_from_slice(bytes);
        self.more.notify_all();
    }

    /// No more is coming.
    pub fn finish(&self) {
        self.lock().1 = true;
        self.more.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Vec<u8>, bool)> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Copies what is at `at` into `into`, waiting for it; 0 only at the end.
    pub fn read_at(&self, at: usize, into: &mut [u8]) -> usize {
        let mut state = self.lock();
        while state.0.len() <= at && !state.1 {
            state = self.more.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let available = state.0.len().saturating_sub(at).min(into.len());
        into[..available].copy_from_slice(&state.0[at..at + available]);
        available
    }

    pub fn done(&self) -> bool {
        self.lock().1
    }
}

/// A [`Growing`] as a file the decoder reads (forwards; it never seeks back far).
struct Reader {
    source: Arc<Growing>,
    at: usize,
}

impl Read for Reader {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let read = self.source.read_at(self.at, into);
        self.at += read;
        Ok(read)
    }
}

impl Seek for Reader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.at = match to {
            SeekFrom::Start(at) => at as usize,
            SeekFrom::Current(by) => self.at.checked_add_signed(by as isize).ok_or_else(|| io::Error::other("Seek before the start"))?,
            SeekFrom::End(_) => return Err(io::Error::new(io::ErrorKind::Unsupported, "The end is still downloading")),
        };
        Ok(self.at as u64)
    }
}

impl symphonia::core::io::MediaSource for Reader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// A WAV header for a stream of unknown length (sizes at their maximum,
/// which players read as "until it ends").
fn wav_header(rate: u32, channels: u16) -> Vec<u8> {
    let block = channels * 2;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&u32::MAX.to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes()); // PCM
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&(rate * block as u32).to_le_bytes());
    header.extend_from_slice(&block.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(u32::MAX - 36).to_le_bytes());
    header
}

/// Over 0.9 the peaks are rounded off instead of clipped: the loudness of
/// a stream is not known until its end, so the exciter's extra is caught here.
pub fn soft_limit(sample: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let level = sample.abs();
    if level <= KNEE { sample } else { sample.signum() * (KNEE + (1.0 - KNEE) * ((level - KNEE) / (1.0 - KNEE)).tanh()) }
}

/// Decodes `mp3` as it arrives, excites it and writes 16-bit WAV to `wav`.
pub fn enhance_stream(mp3: Arc<Growing>, wav: &Growing, strength: Strength) -> io::Result<()> {
    let mut exciters: Vec<Exciter> = Vec::new();
    let mut out = Vec::new();
    crate::enhance::decode_stream(Box::new(Reader { source: mp3, at: 0 }), Some("mp3"), |rate, count, interleaved| {
        if exciters.is_empty() {
            exciters = (0..count).map(|_| Exciter::new(rate, strength)).collect();
            wav.push(&wav_header(rate, count as u16));
        }
        if count != exciters.len() {
            return Ok(());
        }
        out.clear();
        for frame in interleaved.chunks_exact(count) {
            for (exciter, sample) in exciters.iter_mut().zip(frame) {
                let level = soft_limit(exciter.run(*sample));
                out.extend_from_slice(&((level * i16::MAX as f32).round() as i16).to_le_bytes());
            }
        }
        wav.push(&out);
        Ok(())
    })
}

/// Serves `wav` to the player: each request from the byte it asks for
/// (`Range: bytes=N-`), waiting for what has not been made yet.
fn serve(mut stream: TcpStream, wav: &Growing) -> io::Result<()> {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") && request.len() < 16 * 1024 {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        request.push(byte[0]);
    }
    let request = String::from_utf8_lossy(&request);
    let from = request
        .lines()
        .find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string))
        .and_then(|range| range.split('-').next()?.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: audio/wav\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n")?;
    let mut at = from;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = wav.read_at(at, &mut buffer);
        if read == 0 {
            return Ok(());
        }
        stream.write_all(&buffer[..read])?;
        at += read;
    }
}

/// Downloads track `id` from `url` in the background: its MP3 as it
/// arrives (into the cache too). With `enhance`, the full enhancement is
/// cached as FLAC once the whole track is in, for later plays.
pub fn download(id: &str, url: &str, enhance: Option<Strength>, progress: Arc<Progress>) -> Arc<Growing> {
    let mp3 = Arc::new(Growing::default());
    let (id, url, arriving) = (id.to_string(), url.to_string(), mp3.clone());
    std::thread::spawn(move || {
        let fetched = yandex::fetch_with(&id, &url, &progress, |piece| arriving.push(piece));
        arriving.finish();
        if let (Ok(_), Some(strength)) = (fetched, enhance) {
            let _ = yandex::enhanced(&id, strength);
        }
    });
    mp3
}

/// What is arriving in `source`, for [`crate::enhance::decode_stream`].
pub fn reader(source: Arc<Growing>) -> Box<dyn MediaSource> {
    Box::new(Reader { source, at: 0 })
}

/// Starts enhanced streaming of track `id` from `url` at `strength`: the
/// local address to play, at once. Downloading, enhancing and serving go
/// on in the background; `progress` fills as the MP3 arrives.
pub fn start(id: &str, url: &str, strength: Strength, progress: Arc<Progress>) -> io::Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = format!("http://127.0.0.1:{}/{id}.wav", listener.local_addr()?.port());
    let mp3 = download(id, url, Some(strength), progress);
    let wav = Arc::new(Growing::default());

    let made = wav.clone();
    std::thread::spawn(move || {
        if let Err(error) = enhance_stream(mp3, &made, strength) {
            crate::log("Yandex Music", format!("enhanced stream stopped: {error}"));
        }
        made.finish();
    });

    std::thread::spawn(move || {
        // Serves until the stream is made and nobody has asked for a while.
        let _ = listener.set_nonblocking(true);
        let open = Arc::new(AtomicUsize::new(0));
        let mut idle = Duration::ZERO;
        const TICK: Duration = Duration::from_millis(100);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    idle = Duration::ZERO;
                    let _ = stream.set_nonblocking(false);
                    let (wav, open) = (wav.clone(), open.clone());
                    open.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let _ = serve(stream, &wav);
                        open.fetch_sub(1, Ordering::SeqCst);
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(TICK);
                    if open.load(Ordering::SeqCst) == 0 {
                        idle += TICK;
                    }
                    // A skipped track is never asked for again: stop after 2 minutes alone.
                    if idle >= Duration::from_secs(if wav.done() { 30 } else { 120 }) {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_read_while_it_grows() {
        let growing = Arc::new(Growing::default());
        let writer = growing.clone();
        let feeding = std::thread::spawn(move || {
            for piece in [b"abc".as_slice(), b"def"] {
                std::thread::sleep(Duration::from_millis(20));
                writer.push(piece);
            }
            writer.finish();
        });
        let mut all = Vec::new();
        Reader { source: growing, at: 0 }.read_to_end(&mut all).unwrap();
        feeding.join().unwrap();
        assert_eq!(all, b"abcdef", "the reader waited for the second piece");
    }

    #[test]
    fn an_mp3_arriving_in_pieces_comes_out_as_enhanced_wav() {
        let rate = 44_100;
        let tone: Vec<f32> = (0..rate as usize).map(|index| 0.5 * (std::f32::consts::TAU * 440.0 * index as f32 / rate as f32).sin()).collect();
        let dir = std::env::temp_dir().join(format!("files-music-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A FLAC stands in for the MP3: the decoder probes either.
        let file = dir.join("tone.flac");
        crate::enhance::encode_flac(&crate::enhance::Audio { rate, channels: vec![tone.clone(), tone] }, &file).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        let mp3 = Arc::new(Growing::default());
        let feeder = mp3.clone();
        let feeding = std::thread::spawn(move || {
            for piece in bytes.chunks(4096) {
                feeder.push(piece);
                std::thread::sleep(Duration::from_millis(1));
            }
            feeder.finish();
        });
        let wav = Growing::default();
        enhance_stream(mp3, &wav, Strength::Medium).unwrap();
        feeding.join().unwrap();
        let made = wav.lock().0.clone();
        assert_eq!(&made[..4], b"RIFF");
        assert_eq!(made.len(), 44 + rate as usize * 2 * 2, "every frame, 16-bit stereo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_player_gets_the_wav_over_http_from_any_byte() {
        let wav = Arc::new(Growing::default());
        wav.push(b"0123456789");
        wav.finish();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let serving = wav.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let _ = serve(stream.unwrap(), &serving);
            }
        });
        let get = |extra: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(stream, "GET /t.wav HTTP/1.1\r\nHost: x\r\n{extra}\r\n").unwrap();
            let mut answer = String::new();
            stream.read_to_string(&mut answer).unwrap();
            answer
        };
        assert!(get("").ends_with("\r\n\r\n0123456789"));
        let ranged = get("Range: bytes=4-\r\n");
        assert!(ranged.starts_with("HTTP/1.1 206") && ranged.ends_with("\r\n\r\n456789"), "{ranged}");
    }
}
