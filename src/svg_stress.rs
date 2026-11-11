//! SVG stress test: 5,000 SVGs with heavy filters, rasterized the way a
//! folder shows them. `cargo test --release --bin file-force svg_stress -- --ignored --nocapture --test-threads=1`

#[cfg(test)]
mod tests {
    use std::time::Instant;

    /// One SVG with blur, turbulence, a drop shadow and a colour matrix.
    pub(crate) fn filtered_svg(index: usize, size: u32) -> String {
        let hue = (index * 37) % 360;
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 512 512">
<defs>
<filter id="blur" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="{blur}"/></filter>
<filter id="noise"><feTurbulence type="fractalNoise" baseFrequency="0.0{freq}" numOctaves="4" seed="{index}"/><feColorMatrix type="hueRotate" values="{hue}"/><feComposite operator="in" in2="SourceGraphic"/></filter>
<filter id="shadow"><feDropShadow dx="8" dy="12" stdDeviation="10" flood-color="#000" flood-opacity="0.5"/></filter>
<linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl({hue},80%,60%)"/><stop offset="1" stop-color="hsl({hue2},80%,40%)"/></linearGradient>
</defs>
<rect width="512" height="512" fill="url(#g)"/>
<rect width="512" height="512" filter="url(#noise)" opacity="0.6"/>
<circle cx="256" cy="230" r="140" fill="hsl({hue2},90%,70%)" filter="url(#blur)"/>
<g filter="url(#shadow)"><rect x="120" y="300" width="272" height="120" rx="24" fill="#fff" opacity="0.85"/></g>
<text x="256" y="375" font-size="48" text-anchor="middle" font-family="sans-serif" fill="#222">#{index}</text>
</svg>"##,
            blur = 6 + index % 20,
            freq = 1 + index % 9,
            hue2 = (hue + 120) % 360,
        )
    }

    fn render(svg: &str, width: u32) -> (std::time::Duration, usize) {
        let started = Instant::now();
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
        let scale = width as f32 / tree.size().width();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, (tree.size().height() * scale) as u32).unwrap();
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        (started.elapsed(), pixmap.data().len())
    }

    fn percentile(times: &mut [f64], p: f64) -> f64 {
        times.sort_by(|a, b| a.total_cmp(b));
        times[((times.len() as f64 - 1.) * p) as usize]
    }

    #[test]
    #[ignore]
    fn svg_stress() {
        let dir = std::env::temp_dir().join(format!("file-force-svg-stress-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let started = Instant::now();
        for index in 0..5000 {
            std::fs::write(dir.join(format!("art-{index:04}.svg")), filtered_svg(index, 1024)).unwrap();
        }
        println!("5,000 filtered SVGs written in {:?} → {}", started.elapsed(), dir.display());
        // A screen of cards: ~30 at once.
        for (label, width) in [("as GPUI draws an SVG (its own size, 1024 px)", 1024u32), ("as a 320 px thumbnail", 320)] {
            let mut times = Vec::new();
            let mut bytes = 0;
            for index in 0..60 {
                let (time, size) = render(&filtered_svg(index, 1024), width);
                times.push(time.as_secs_f64() * 1000.);
                bytes = size;
            }
            let screen: f64 = times.iter().take(30).sum();
            println!(
                "{label}: p50 {:.1} ms · p95 {:.1} ms per SVG · a 30-card screen {:.0} ms of work · {:.1} MB per image kept",
                percentile(&mut times.clone(), 0.5),
                percentile(&mut times, 0.95),
                screen,
                bytes as f64 / 1_048_576.
            );
        }
        // The real pipeline: each SVG once to a 320 px PNG, then a screen is 30 small PNGs.
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let started = Instant::now();
        for index in 0..30 {
            crate::thumbnails::rasterize_svg(&dir.join(format!("art-{index:04}.svg")), &cache.join(format!("{index}.svg.png"))).unwrap();
        }
        let cold = started.elapsed();
        let started = Instant::now();
        let mut bytes = 0;
        for index in 0..30 {
            let image = image::open(cache.join(format!("{index}.svg.png"))).unwrap();
            bytes += image.width() * image.height() * 4;
        }
        println!("Files' cache: first visit of a 30-card screen {cold:?} (background worker), every later visit {:?} to decode, {:.1} MB on screen", started.elapsed(), bytes as f64 / 1_048_576.);
        println!("memory if every one were kept at 1024 px: {:.1} GB; at 320 px: {:.0} MB", 5000. * 4. / 1024., 5000. * 0.39);
    }
}
