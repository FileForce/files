//! Mail: several IMAP/SMTP accounts (Gmail, Yandex or any server), reading
//! mail with its attachments and sending files as attachments.
//!
//! Sign-in is an app password, kept in the system keychain; accounts
//! themselves (address, servers) are listed in `mail.toml` next to
//! `settings.toml`. TLS is rustls with the Mozilla roots on every system.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use files_sdk::{Icon, Manifest, Permission, host};

pub static MANIFEST: Manifest = Manifest {
    id: "mail",
    name: "Mail",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in (experimental)",
    summary: "Your mailboxes in the sidebar: read mail, save attachments into any folder, send files as attachments. Gmail, Yandex or any IMAP server, several accounts at once.",
    icon: Icon::Builtin("mail"),
    adds: &[
        "Sidebar → Mail: every account and its folders",
        "Context menu on a file → Send as Attachment…",
        "Mail window: read, reply, write, save attachments",
        "Global search → email:text, email:from:name, email:to:address",
    ],
    permissions: &[Permission::Network, Permission::Secrets, Permission::ReadFiles, Permission::WriteFiles],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: Some("blue"),
    tier: files_sdk::Tier::Free,
};

pub struct Mail;

pub static EXTENSION: Mail = Mail;

impl files_sdk::Extension for Mail {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn search_keys(&self) -> &'static [files_sdk::SearchKey] {
        &[files_sdk::SearchKey {
            key: "email",
            summary: "mail text, sender, recipient or subject (partial matches)",
            example: "email:to:ann",
        }]
    }
}

/// How long a server may stay silent before we give up.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Messages listed per folder (newest first).
pub const PAGE: u32 = 50;

// ---------------------------------------------------------------- accounts

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Gmail,
    Yandex,
    #[default]
    Other,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::Gmail, Self::Yandex, Self::Other];

    pub fn name(self) -> &'static str {
        match self {
            Self::Gmail => "Google",
            Self::Yandex => "Yandex",
            Self::Other => "Other",
        }
    }

    /// Guessed from the address: gmail.com → Google, yandex.* / ya.ru → Yandex.
    pub fn of(email: &str) -> Self {
        let domain = email.rsplit('@').next().unwrap_or_default().to_lowercase();
        match domain.as_str() {
            "gmail.com" | "googlemail.com" => Self::Gmail,
            "ya.ru" | "yandex.ru" | "yandex.com" | "yandex.by" | "yandex.kz" | "yandex.ua" | "narod.ru" => Self::Yandex,
            _ => Self::Other,
        }
    }

    /// IMAP and SMTP over implicit TLS.
    pub fn servers(self) -> Option<(Server, Server)> {
        match self {
            Self::Gmail => Some((Server::new("imap.gmail.com", 993), Server::new("smtp.gmail.com", 465))),
            Self::Yandex => Some((Server::new("imap.yandex.com", 993), Server::new("smtp.yandex.com", 465))),
            Self::Other => None,
        }
    }

    /// Where to create the app password used to sign in.
    pub fn app_password_url(self) -> Option<&'static str> {
        match self {
            Self::Gmail => Some("https://myaccount.google.com/apppasswords"),
            Self::Yandex => Some("https://id.yandex.ru/security/app-passwords"),
            Self::Other => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
}

impl Server {
    pub fn new(host: &str, port: u16) -> Self {
        Self { host: host.to_string(), port }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Account {
    /// The address; also the keychain entry of its password.
    pub email: String,
    /// Shown as the sender ("Ann Lee <ann@…>"); may be empty.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub provider: Provider,
    pub imap: Server,
    pub smtp: Server,
}

impl Account {
    /// An account on a known provider, or `None` for "Other" (enter servers).
    pub fn for_provider(email: &str, name: &str, provider: Provider) -> Option<Self> {
        let (imap, smtp) = provider.servers()?;
        Some(Self { email: email.trim().to_string(), name: name.trim().to_string(), provider, imap, smtp })
    }

    fn mailbox(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { format!("{} <{}>", self.name, self.email) }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Store {
    #[serde(default, rename = "account")]
    accounts: Vec<Account>,
}

fn store_path() -> Option<PathBuf> {
    Some(host().config_dir()?.join("mail.toml"))
}

/// Every account, in the order they were added.
pub fn accounts() -> Vec<Account> {
    store_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| toml::from_str::<Store>(&text).ok())
        .map(|store| store.accounts)
        .unwrap_or_default()
}

fn save(accounts: Vec<Account>) -> io::Result<()> {
    let path = store_path().ok_or_else(|| io::Error::other("no settings folder"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(&Store { accounts }).map_err(io::Error::other)?;
    std::fs::write(path, text)
}

/// Checks the password against the server, then remembers the account.
pub fn add_account(account: Account, password: &str) -> io::Result<()> {
    if !account.email.contains('@') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Enter the full address, e.g. you@gmail.com"));
    }
    let password: String = password.chars().filter(|c| !c.is_whitespace()).collect();
    // Google shows app passwords in groups of four: spaces are not part of it.
    login(&account, &password)?.logout().ok();
    files_sdk::secrets::set(MANIFEST.id, &account.email, &password)?;
    let mut all: Vec<_> = accounts().into_iter().filter(|known| known.email != account.email).collect();
    all.push(account.clone());
    save(all)?;
    files_sdk::log::record(MANIFEST.id, files_sdk::log::Kind::Network, format!("Mail account added: {}", account.email), None);
    Ok(())
}

pub fn remove_account(email: &str) -> io::Result<()> {
    let _ = files_sdk::secrets::delete(MANIFEST.id, email);
    save(accounts().into_iter().filter(|account| account.email != email).collect())
}

fn password(account: &Account) -> io::Result<String> {
    files_sdk::secrets::get(MANIFEST.id, &account.email)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, format!("No password saved for {}", account.email)))
}

// ------------------------------------------------------------------- TLS

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("ring supports the default TLS versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

/// Connects to the first address that answers, IPv4 first: many networks
/// resolve IPv6 but cannot reach it ("Network is unreachable").
fn connect_tcp(server: &Server) -> io::Result<TcpStream> {
    use std::net::ToSocketAddrs as _;
    let mut addresses: Vec<_> = (server.host.as_str(), server.port).to_socket_addrs()?.collect();
    addresses.sort_by_key(|address| address.is_ipv6());
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{} not found", server.host));
    for address in addresses {
        match TcpStream::connect_timeout(&address, Duration::from_secs(10)) {
            Ok(tcp) => return Ok(tcp),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// The address [`connect_tcp`] would use, for SMTP (lettre connects itself).
fn reachable_ip(server: &Server) -> io::Result<std::net::IpAddr> {
    Ok(connect_tcp(server)?.peer_addr()?.ip())
}

fn connect_tls(server: &Server) -> io::Result<Tls> {
    let tcp = connect_tcp(server)?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    let name = rustls::pki_types::ServerName::try_from(server.host.clone()).map_err(io::Error::other)?;
    let connection = rustls::ClientConnection::new(tls_config(), name).map_err(io::Error::other)?;
    Ok(rustls::StreamOwned::new(connection, tcp))
}

// ------------------------------------------------------------------ IMAP

type Session = imap::Session<Tls>;

fn imap_error(error: imap::Error) -> io::Error {
    match error {
        imap::Error::Io(error) => error,
        imap::Error::No(response) => io::Error::new(io::ErrorKind::PermissionDenied, format!("The server refused: {}", response.information)),
        imap::Error::Bad(response) => io::Error::new(io::ErrorKind::PermissionDenied, format!("The server refused: {}", response.information)),
        other => io::Error::other(other.to_string()),
    }
}

fn login(account: &Account, password: &str) -> io::Result<Session> {
    let started = std::time::Instant::now();
    let stream = connect_tls(&account.imap)?;
    let mut client = imap::Client::new(stream);
    client.read_greeting().map_err(imap_error)?;
    let session = client.login(&account.email, password).map_err(|(error, _)| {
        let error = imap_error(error);
        if error.kind() == io::ErrorKind::PermissionDenied {
            let hint = account.provider.app_password_url().map_or(String::new(), |url| format!(" Use an app password from {url}"));
            io::Error::new(io::ErrorKind::PermissionDenied, format!("Wrong address or password.{hint}"))
        } else {
            error
        }
    })?;
    files_sdk::log::record(
        MANIFEST.id,
        files_sdk::log::Kind::Network,
        format!("IMAP {} · signed in as {}", account.imap.host, account.email),
        Some(format!("{} ms", started.elapsed().as_millis())),
    );
    Ok(session)
}

// One signed-in connection per account, reused: signing in takes longer
// than reading a message. A broken connection is replaced once.
fn pool() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, Session>> {
    static POOL: std::sync::OnceLock<Mutex<std::collections::HashMap<String, Session>>> = std::sync::OnceLock::new();
    POOL.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_session<T>(account: &Account, mut run: impl FnMut(&mut Session) -> imap::Result<T>) -> io::Result<T> {
    let pooled = pool().remove(&account.email);
    let reused = pooled.is_some();
    let mut session = match pooled {
        Some(session) => session,
        None => login(account, &password(account)?)?,
    };
    let result = match run(&mut session) {
        // A dropped connection: sign in again and retry once.
        Err(imap::Error::Io(_) | imap::Error::ConnectionLost) if reused => {
            session = login(account, &password(account)?)?;
            run(&mut session)
        }
        other => other,
    };
    match result {
        Ok(value) => {
            pool().insert(account.email.clone(), session);
            Ok(value)
        }
        Err(error) => Err(imap_error(error)),
    }
}

/// Folders with a role (RFC 6154), in the order they are listed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Special {
    Inbox,
    Flagged,
    Drafts,
    Sent,
    /// Messages waiting to be sent later (kept on this computer).
    Scheduled,
    Archive,
    All,
    Junk,
    Trash,
}

impl Special {
    pub const ALL: [Self; 9] =
        [Self::Inbox, Self::Flagged, Self::Drafts, Self::Sent, Self::Scheduled, Self::Archive, Self::All, Self::Junk, Self::Trash];

    pub fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Flagged => "Starred",
            Self::Drafts => "Drafts",
            Self::Sent => "Sent",
            Self::Scheduled => "Scheduled",
            Self::Archive => "Archive",
            Self::All => "All Mail",
            Self::Junk => "Spam",
            Self::Trash => "Trash",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Flagged => "flagged",
            Self::Drafts => "drafts",
            Self::Sent => "sent",
            Self::Scheduled => "scheduled",
            Self::Archive => "archive",
            Self::All => "all",
            Self::Junk => "junk",
            Self::Trash => "trash",
        }
    }

    /// The role from the server's flags, or from a well-known name when the
    /// server sends none (older servers, some Yandex folders).
    fn of(name: &str, attributes: &[imap_proto::NameAttribute]) -> Option<Self> {
        use imap_proto::NameAttribute as A;
        if name.eq_ignore_ascii_case("INBOX") {
            return Some(Self::Inbox);
        }
        for attribute in attributes {
            let special = match attribute {
                A::Flagged => Self::Flagged,
                A::Drafts => Self::Drafts,
                A::Sent => Self::Sent,
                A::Archive => Self::Archive,
                A::All => Self::All,
                A::Junk => Self::Junk,
                A::Trash => Self::Trash,
                _ => continue,
            };
            return Some(special);
        }
        let last = name.rsplit(['/', '.', '|']).next().unwrap_or(name).to_lowercase();
        Some(match last.as_str() {
            "sent" | "sent items" | "sent mail" | "отправленные" => Self::Sent,
            "drafts" | "черновики" => Self::Drafts,
            "trash" | "deleted" | "deleted items" | "bin" | "удаленные" | "удалённые" | "корзина" => Self::Trash,
            "spam" | "junk" | "спам" => Self::Junk,
            "archive" | "архив" => Self::Archive,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    /// The server's name, passed back to [`messages`].
    pub name: String,
    /// The last part, for display ("[Gmail]/Sent Mail" → "Sent Mail").
    pub label: String,
    pub special: Option<Special>,
}

/// Mail folders: the special ones first (Inbox, Starred, Drafts, Sent…),
/// then the rest by name; containers that hold no mail are left out.
pub fn folders(account: &Account) -> io::Result<Vec<Folder>> {
    let names = with_session(account, |session| {
        Ok(session
            .list(Some(""), Some("*"))?
            .iter()
            .filter(|name| !name.attributes().iter().any(|attribute| matches!(attribute, imap_proto::NameAttribute::NoSelect)))
            .map(|name| {
                let delimiter = name.delimiter().unwrap_or("/");
                let special = Special::of(name.name(), name.attributes());
                let label = name.name().rsplit(delimiter).next().unwrap_or(name.name()).to_string();
                Folder { name: name.name().to_string(), label: if special == Some(Special::Inbox) { "Inbox".into() } else { label }, special }
            })
            .collect::<Vec<_>>())
    })?;
    let mut folders = names;
    folders.sort_by(|a, b| (a.special.is_none(), a.special, a.label.to_lowercase()).cmp(&(b.special.is_none(), b.special, b.label.to_lowercase())));
    Ok(folders)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub uid: u32,
    pub from: String,
    /// The sender's address, e.g. to copy it.
    pub address: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    /// Seconds since the Unix epoch, when the message has a date.
    pub date: Option<i64>,
    pub seen: bool,
    pub size: u32,
}

type Key = (String, String);

#[derive(Default)]
struct Cache {
    lists: std::collections::HashMap<Key, Vec<Summary>>,
    bodies: std::collections::HashMap<(String, String, u32), Message>,
}

/// Bodies kept in memory; the oldest are dropped past this.
const CACHED_BODIES: usize = 400;

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static CACHE: std::sync::OnceLock<Mutex<Cache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// (messages in memory, bytes of their text and attachments), for the developer tools.
pub fn cache_stats() -> (usize, u64) {
    let cache = cache();
    let bytes = cache
        .bodies
        .values()
        .map(|message| (message.body.len() + message.html.as_ref().map_or(0, String::len) + message.attachments.iter().map(|attachment| attachment.bytes.len()).sum::<usize>()) as u64)
        .sum();
    (cache.bodies.len(), bytes)
}

/// The last list read for `folder`, shown at once while it is refreshed.
pub fn cached_messages(account: &Account, folder: &str) -> Option<Vec<Summary>> {
    cache().lists.get(&(account.email.clone(), folder.to_string())).cloned()
}

pub fn cached_message(account: &Account, folder: &str, uid: u32) -> Option<Message> {
    cache().bodies.get(&(account.email.clone(), folder.to_string(), uid)).cloned()
}

const SUMMARY_FIELDS: &str = "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE)])";

fn summaries(fetched: &imap::types::Fetches) -> Vec<Summary> {
    let mut list: Vec<Summary> = fetched
        .iter()
        .filter_map(|message| {
            let header = parse_summary(message.header().or_else(|| message.body()).unwrap_or_default());
            Some(Summary {
                uid: message.uid?,
                seen: message.flags().iter().any(|flag| matches!(flag, imap::types::Flag::Seen)),
                size: message.size.unwrap_or(0),
                ..header
            })
        })
        .collect();
    list.sort_by(|a, b| b.uid.cmp(&a.uid));
    list
}

/// The newest [`PAGE`] messages of `folder`, newest first.
pub fn messages(account: &Account, folder: &str) -> io::Result<Vec<Summary>> {
    let list = with_session(account, |session| {
        let mailbox = session.examine(folder)?;
        if mailbox.exists == 0 {
            return Ok(Vec::new());
        }
        let first = mailbox.exists.saturating_sub(PAGE - 1).max(1);
        Ok(summaries(&session.fetch(format!("{first}:{}", mailbox.exists), SUMMARY_FIELDS)?))
    })?;
    cache().lists.insert((account.email.clone(), folder.to_string()), list.clone());
    Ok(list)
}

/// From, subject and date out of raw header lines (MIME words decoded).
pub fn parse_summary(header: &[u8]) -> Summary {
    let parsed = mail_parser::MessageParser::default().parse_headers(header);
    let sender = parsed.as_ref().and_then(|message| message.from()).and_then(|from| from.first());
    let address = sender.as_ref().and_then(|sender| sender.address()).unwrap_or_default().to_string();
    let from = sender.as_ref().and_then(|sender| sender.name()).map_or_else(|| address.clone(), str::to_string);
    let to = parsed
        .as_ref()
        .and_then(|message| message.to())
        .map(|list| list.mailboxes().filter_map(|address| address.address()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let cc = parsed
        .as_ref()
        .and_then(|message| message.cc())
        .map(|list| list.mailboxes().filter_map(|address| address.address()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    Summary {
        uid: 0,
        from,
        address,
        to,
        cc,
        subject: parsed.as_ref().and_then(|message| message.subject()).unwrap_or("(no subject)").to_string(),
        date: parsed.as_ref().and_then(|message| message.date()).map(|date| date.to_timestamp()),
        seen: false,
        size: 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub uid: u32,
    pub from: String,
    /// The sender's address, to reply to.
    pub reply_to: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub date: Option<i64>,
    /// Plain text (HTML-only mail is turned into text).
    pub body: String,
    /// The HTML version when the message has one (styles, line breaks, links).
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

/// The whole message `uid` of `folder`, from memory when it was read
/// before. Reading marks it as seen.
pub fn message(account: &Account, folder: &str, uid: u32) -> io::Result<Message> {
    if let Some(message) = cached_message(account, folder, uid) {
        return Ok(message);
    }
    let raw = with_session(account, |session| {
        session.select(folder)?;
        let fetched = session.uid_fetch(uid.to_string(), "BODY[]")?;
        Ok(fetched.iter().find_map(|message| message.body().map(<[u8]>::to_vec)))
    })?
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "The message is gone"))?;
    let mut message = parse_message(&raw).ok_or_else(|| io::Error::other("This message cannot be read"))?;
    message.uid = uid;
    let mut cache = cache();
    if cache.bodies.len() >= CACHED_BODIES {
        cache.bodies.clear();
    }
    cache.bodies.insert((account.email.clone(), folder.to_string(), uid), message.clone());
    if let Some(list) = cache.lists.get_mut(&(account.email.clone(), folder.to_string()))
        && let Some(summary) = list.iter_mut().find(|summary| summary.uid == uid)
    {
        summary.seen = true;
    }
    Ok(message)
}

/// Reads the newest messages ahead, so opening them is instant.
pub fn prefetch(account: &Account, folder: &str, uids: &[u32]) {
    for &uid in uids {
        if cached_message(account, folder, uid).is_none() && message(account, folder, uid).is_err() {
            break;
        }
    }
}

// ------------------------------------------------------------ changing mail

/// Marks a message read or unread.
pub fn set_seen(account: &Account, folder: &str, uid: u32, seen: bool) -> io::Result<()> {
    store_flag(account, folder, uid, "\\Seen", seen)?;
    update_cached(account, folder, uid, |summary| summary.seen = seen);
    Ok(())
}

/// Stars (flags) a message or takes the star away.
pub fn set_flagged(account: &Account, folder: &str, uid: u32, flagged: bool) -> io::Result<()> {
    store_flag(account, folder, uid, "\\Flagged", flagged)
}

fn store_flag(account: &Account, folder: &str, uid: u32, flag: &str, on: bool) -> io::Result<()> {
    with_session(account, |session| {
        session.select(folder)?;
        session.uid_store(uid.to_string(), format!("{}FLAGS ({flag})", if on { "+" } else { "-" }))?;
        Ok(())
    })
}

fn update_cached(account: &Account, folder: &str, uid: u32, change: impl Fn(&mut Summary)) {
    if let Some(list) = cache().lists.get_mut(&(account.email.clone(), folder.to_string()))
        && let Some(summary) = list.iter_mut().find(|summary| summary.uid == uid)
    {
        change(summary);
    }
}

fn forget(account: &Account, folder: &str, uid: u32) {
    let mut cache = cache();
    if let Some(list) = cache.lists.get_mut(&(account.email.clone(), folder.to_string())) {
        list.retain(|summary| summary.uid != uid);
    }
    cache.bodies.remove(&(account.email.clone(), folder.to_string(), uid));
}

/// Moves a message into `target` (MOVE, or copy + delete on old servers).
pub fn move_to(account: &Account, folder: &str, uid: u32, target: &str) -> io::Result<()> {
    with_session(account, |session| {
        session.select(folder)?;
        let can_move = session.capabilities().map(|capabilities| capabilities.has_str("MOVE")).unwrap_or(false);
        if can_move {
            session.uid_mv(uid.to_string(), target)?;
        } else {
            session.uid_copy(uid.to_string(), target)?;
            session.uid_store(uid.to_string(), "+FLAGS (\\Deleted)")?;
            session.expunge()?;
        }
        Ok(())
    })?;
    forget(account, folder, uid);
    Ok(())
}

/// Deletes for good (used inside Trash, or when there is no Trash folder).
pub fn delete_forever(account: &Account, folder: &str, uid: u32) -> io::Result<()> {
    with_session(account, |session| {
        session.select(folder)?;
        session.uid_store(uid.to_string(), "+FLAGS (\\Deleted)")?;
        session.expunge()?;
        Ok(())
    })?;
    forget(account, folder, uid);
    Ok(())
}

// ---------------------------------------------------------------- search

/// A mail search: `from:ann to:bob@x.com subject:invoice is:unread words`.
/// `email:you@gmail.com` picks the account; filters may be chained after it
/// with colons: `email:you@gmail.com:to:someone@x.com`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Search {
    pub account: Option<String>,
    /// IMAP SEARCH keys, already quoted.
    pub criteria: Vec<String>,
    addresses: Vec<(Address, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Address {
    From,
    To,
    Cc,
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Search {
    pub fn parse(query: &str) -> Self {
        let mut search = Self::default();
        for token in query.split_whitespace() {
            let mut rest = token;
            if let Some(after) = rest.strip_prefix("email:") {
                // The address itself has no colon; what follows is `key:value…`.
                let (address, tail) = after.split_once(':').unwrap_or((after, ""));
                search.account = Some(address.to_lowercase());
                rest = tail;
            }
            let parts: Vec<&str> = rest.split(':').filter(|part| !part.is_empty()).collect();
            let mut index = 0;
            while index < parts.len() {
                let (key, value) = (parts[index].to_lowercase(), parts.get(index + 1).copied());
                let address = match (key.as_str(), value) {
                    ("from", Some(value)) => Some((Address::From, value)),
                    ("to", Some(value)) => Some((Address::To, value)),
                    ("cc", Some(value)) => Some((Address::Cc, value)),
                    _ => None,
                };
                if let Some((field, value)) = address {
                    search.addresses.push((field, value.to_lowercase()));
                    index += 2;
                    continue;
                }
                let criterion = match (key.as_str(), value) {
                    ("subject", Some(value)) => Some(format!("SUBJECT {}", quoted(value))),
                    ("is", Some("unread")) => Some("UNSEEN".to_string()),
                    ("is", Some("read")) => Some("SEEN".to_string()),
                    ("is", Some("starred" | "flagged")) => Some("FLAGGED".to_string()),
                    _ => None,
                };
                match criterion {
                    Some(criterion) => {
                        search.criteria.push(criterion);
                        index += 2;
                    }
                    None => {
                        search.criteria.push(format!("TEXT {}", quoted(parts[index])));
                        index += 1;
                    }
                }
            }
        }
        search
    }

    fn matches(&self, summary: &Summary) -> bool {
        self.addresses.iter().all(|(field, needle)| {
            let haystack = match field {
                Address::From => format!("{} {}", summary.from, summary.address),
                Address::To => summary.to.clone(),
                Address::Cc => summary.cc.clone(),
            };
            haystack.to_lowercase().contains(needle)
        })
    }
}

/// Messages of `folder` matching `search`, newest first (at most [`PAGE`]).
pub fn search(account: &Account, folder: &str, search: &Search) -> io::Result<Vec<Summary>> {
    if search.criteria.is_empty() && search.addresses.is_empty() {
        return messages(account, folder);
    }
    let query = if search.criteria.is_empty() { "ALL".to_string() } else { search.criteria.join(" ") };
    let mut found = with_session(account, |session| {
        session.examine(folder)?;
        let mut uids: Vec<u32> = session.uid_search(&query)?.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        if search.addresses.is_empty() {
            uids.truncate(PAGE as usize);
        }
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        Ok(summaries(&session.uid_fetch(set, SUMMARY_FIELDS)?))
    })?;
    found.retain(|summary| search.matches(summary));
    found.truncate(PAGE as usize);
    Ok(found)
}

#[derive(Clone, Debug, PartialEq)]
pub struct MailResult {
    pub account: Account,
    pub folder: Folder,
    pub summary: Summary,
}

/// Searches every folder of every configured account, newest mail first.
pub fn global_search(query: &str) -> io::Result<Vec<MailResult>> {
    let search = Search::parse(query);
    let mut found = Vec::new();
    let mut first_error = None;
    let mut loaded = false;
    for account in accounts() {
        let folders = match folders(&account) {
            Ok(folders) => folders,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        for folder in folders.into_iter().filter(|folder| folder.special != Some(Special::All)) {
            match self::search(&account, &folder.name, &search) {
                Ok(messages) => {
                    loaded = true;
                    found.extend(messages.into_iter().map(|summary| MailResult { account: account.clone(), folder: folder.clone(), summary }));
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
    }
    if !loaded && found.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }
    found.sort_by(|a, b| b.summary.date.unwrap_or(0).cmp(&a.summary.date.unwrap_or(0)));
    found.truncate(PAGE as usize);
    Ok(found)
}

/// Text, sender and attachments of a raw RFC 5322 message.
pub fn parse_message(raw: &[u8]) -> Option<Message> {
    let parsed = mail_parser::MessageParser::default().parse(raw)?;
    let from = parsed.from().and_then(|list| list.first());
    let to = parsed
        .to()
        .map(|list| list.mailboxes().filter_map(|address| address.address()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let cc = parsed
        .cc()
        .map(|list| list.mailboxes().filter_map(|address| address.address()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let reply = parsed.reply_to().and_then(|list| list.first()).or_else(|| from.clone());
    let attachments = parsed
        .attachments()
        .map(|part| Attachment {
            name: part.attachment_name().map_or_else(|| "attachment".to_string(), safe_name),
            bytes: part.decoded().into_owned(),
        })
        .collect();
    Some(Message {
        uid: 0,
        from: from.as_ref().map_or_else(String::new, |address| match (address.name(), address.address()) {
            (Some(name), Some(email)) => format!("{name} <{email}>"),
            (_, Some(email)) => email.to_string(),
            (Some(name), None) => name.to_string(),
            _ => String::new(),
        }),
        cc,
        reply_to: reply.and_then(|address| address.address().map(str::to_string)).unwrap_or_default(),
        to,
        subject: parsed.subject().unwrap_or("(no subject)").to_string(),
        date: parsed.date().map(|date| date.to_timestamp()),
        body: parsed.body_text(0).map(|text| text.trim().to_string()).unwrap_or_default(),
        html: parsed
            .html_body()
            .any(|part| part.is_content_type("text", "html"))
            .then(|| parsed.body_html(0).map(|html| html.into_owned()))
            .flatten(),
        attachments,
    })
}

/// An attachment's name with no folders in it, so saving stays in the folder.
fn safe_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    if name.is_empty() || name == "." || name == ".." { "attachment".to_string() } else { name }
}

/// Writes an attachment into `folder`, never over an existing file.
pub fn save_attachment(attachment: &Attachment, folder: &Path) -> io::Result<PathBuf> {
    let path = files_sdk::fs::unique_path(&folder.join(safe_name(&attachment.name)));
    std::fs::write(&path, &attachment.bytes)?;
    Ok(path)
}

// ------------------------------------------------------------------ SMTP

#[derive(Clone, Debug, Default, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Draft {
    /// Comma-separated addresses.
    pub to: String,
    #[serde(default)]
    pub cc: String,
    /// Hidden recipients: they get the message, nobody sees them.
    #[serde(default)]
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<PathBuf>,
}

/// Turns a draft into a MIME message from `account` (no network).
pub fn build(account: &Account, draft: &Draft) -> io::Result<lettre::Message> {
    use lettre::message::{Attachment as Part, Mailbox, MultiPart, SinglePart, header::ContentType};
    let invalid = |what: &str, error: &dyn std::fmt::Display| io::Error::new(io::ErrorKind::InvalidInput, format!("{what}: {error}"));
    let from: Mailbox = account.mailbox().parse().map_err(|error| invalid("Sender", &error))?;
    let mut builder = lettre::Message::builder().from(from).subject(draft.subject.trim());
    let list = |text: &str| text.split([',', ';']).map(str::trim).filter(|address| !address.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let (to, cc, bcc) = (list(&draft.to), list(&draft.cc), list(&draft.bcc));
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Add at least one recipient"));
    }
    for address in &to {
        builder = builder.to(address.parse().map_err(|error| invalid(address, &error))?);
    }
    for address in &cc {
        builder = builder.cc(address.parse().map_err(|error| invalid(address, &error))?);
    }
    // Bcc goes into the envelope only; lettre leaves the header out.
    for address in &bcc {
        builder = builder.bcc(address.parse().map_err(|error| invalid(address, &error))?);
    }
    let text = SinglePart::plain(draft.body.clone());
    let message = if draft.attachments.is_empty() {
        builder.singlepart(text)
    } else {
        let mut parts = MultiPart::mixed().singlepart(text);
        for path in &draft.attachments {
            let name = path.file_name().map_or_else(|| "attachment".to_string(), |name| name.to_string_lossy().into_owned());
            let bytes = std::fs::read(path).map_err(|error| io::Error::new(error.kind(), format!("{name}: {error}")))?;
            parts = parts.singlepart(Part::new(name).body(bytes, ContentType::parse("application/octet-stream").expect("valid type")));
        }
        builder.multipart(parts)
    };
    message.map_err(|error| invalid("Message", &error))
}

/// Sends a draft through the account's SMTP server.
pub fn send(account: &Account, draft: &Draft) -> io::Result<()> {
    use lettre::Transport as _;
    let message = build(account, draft)?;
    let password = password(account)?;
    let started = std::time::Instant::now();
    use lettre::transport::smtp::client::{Tls as SmtpTls, TlsParameters};
    // The configured port, then the other submission port: some networks
    // block one of them. Connect to a reachable address, check the name.
    let other = if account.smtp.port == 587 { 465 } else { 587 };
    let (server, ip) = [account.smtp.port, other]
        .into_iter()
        .map(|port| Server::new(&account.smtp.host, port))
        .find_map(|server| reachable_ip(&server).ok().map(|ip| (server, ip)))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!(
                    "Cannot reach {} on ports {} and {other}: this network blocks outgoing mail (reading still works). Try another network or a VPN.",
                    account.smtp.host, account.smtp.port
                ),
            )
        })?;
    let tls = TlsParameters::new(server.host.clone()).map_err(io::Error::other)?;
    let transport = lettre::SmtpTransport::builder_dangerous(ip.to_string())
        .port(server.port)
        .tls(if server.port == 587 { SmtpTls::Required(tls) } else { SmtpTls::Wrapper(tls) })
        .timeout(Some(TIMEOUT))
        .credentials(lettre::transport::smtp::authentication::Credentials::new(account.email.clone(), password))
        .build();
    transport.send(&message).map_err(|error| io::Error::other(format!("Not sent: {error}")))?;
    files_sdk::log::record(
        MANIFEST.id,
        files_sdk::log::Kind::Network,
        format!("SMTP {} · sent \"{}\" to {}", account.smtp.host, draft.subject, draft.to),
        Some(format!("{} attachment(s) · {} ms", draft.attachments.len(), started.elapsed().as_millis())),
    );
    Ok(())
}

// ---------------------------------------------------------------- outbox

/// A message waiting to be sent later. Its files are copied when it is
/// scheduled, so changing or deleting the originals does not change it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Scheduled {
    pub id: u64,
    pub account: String,
    /// Seconds since the Unix epoch.
    pub send_at: i64,
    pub draft: Draft,
    /// Why the last try failed; it is tried again on the next check.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Outbox {
    #[serde(default, rename = "message")]
    messages: Vec<Scheduled>,
}

fn outbox_dir() -> Option<PathBuf> {
    Some(host().config_dir()?.join("mail-outbox"))
}

fn outbox_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_outbox() -> Vec<Scheduled> {
    outbox_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("outbox.toml")).ok())
        .and_then(|text| toml::from_str::<Outbox>(&text).ok())
        .map(|outbox| outbox.messages)
        .unwrap_or_default()
}

fn write_outbox(messages: Vec<Scheduled>) -> io::Result<()> {
    let dir = outbox_dir().ok_or_else(|| io::Error::other("no settings folder"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("outbox.toml"), toml::to_string_pretty(&Outbox { messages }).map_err(io::Error::other)?)
}

/// Every scheduled message, soonest first.
pub fn scheduled() -> Vec<Scheduled> {
    let _lock = outbox_lock();
    let mut messages = read_outbox();
    messages.sort_by_key(|message| message.send_at);
    messages
}

/// Queues `draft` to go out from `account` at `send_at`.
pub fn schedule(account: &Account, draft: &Draft, send_at: i64) -> io::Result<Scheduled> {
    build(account, draft)?;
    let _lock = outbox_lock();
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos() as u64);
    let folder = outbox_dir().ok_or_else(|| io::Error::other("no settings folder"))?.join(id.to_string());
    std::fs::create_dir_all(&folder)?;
    let mut copy = draft.clone();
    copy.attachments = draft
        .attachments
        .iter()
        .map(|path| {
            let target = folder.join(path.file_name().unwrap_or_default());
            std::fs::copy(path, &target).map(|_| target)
        })
        .collect::<io::Result<_>>()?;
    let message = Scheduled { id, account: account.email.clone(), send_at, draft: copy, error: None };
    let mut messages = read_outbox();
    messages.push(message.clone());
    write_outbox(messages)?;
    Ok(message)
}

/// Removes a scheduled message (and its copied files) without sending it.
pub fn cancel(id: u64) -> io::Result<()> {
    let _lock = outbox_lock();
    write_outbox(read_outbox().into_iter().filter(|message| message.id != id).collect())?;
    if let Some(dir) = outbox_dir() {
        let _ = std::fs::remove_dir_all(dir.join(id.to_string()));
    }
    Ok(())
}

/// Sends what is due by `now`. Returns what went out and what failed (a
/// failed one stays queued and is tried again next time).
pub fn send_due(now: i64) -> Vec<(Scheduled, io::Result<()>)> {
    let due: Vec<Scheduled> = scheduled().into_iter().filter(|message| message.send_at <= now).collect();
    let accounts = accounts();
    let mut results = Vec::new();
    for message in due {
        let result = match accounts.iter().find(|account| account.email == message.account) {
            Some(account) => send(account, &message.draft),
            None => Err(io::Error::new(io::ErrorKind::NotFound, format!("The account {} was removed", message.account))),
        };
        if result.is_ok() {
            let _ = cancel(message.id);
        } else {
            let _lock = outbox_lock();
            let error = result.as_ref().err().map(ToString::to_string);
            let messages = read_outbox()
                .into_iter()
                .map(|mut queued| {
                    if queued.id == message.id {
                        queued.error = error.clone();
                    }
                    queued
                })
                .collect();
            let _ = write_outbox(messages);
        }
        results.push((message, result));
    }
    results
}

// Keep the TLS stream usable by `imap`, which needs `Read + Write`.
const _: fn() = || {
    fn stream<T: Read + Write>() {}
    stream::<Tls>();
};

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account::for_provider("ann@gmail.com", "Ann Lee", Provider::Gmail).unwrap()
    }

    #[test]
    fn providers_are_guessed_and_have_servers() {
        assert_eq!(Provider::of("Ann@GMail.com"), Provider::Gmail);
        assert_eq!(Provider::of("ivan@ya.ru"), Provider::Yandex);
        assert_eq!(Provider::of("me@example.org"), Provider::Other);
        let account = account();
        assert_eq!((account.imap.host.as_str(), account.imap.port), ("imap.gmail.com", 993));
        assert_eq!((account.smtp.host.as_str(), account.smtp.port), ("smtp.gmail.com", 465));
        assert!(Account::for_provider("me@example.org", "", Provider::Other).is_none());
    }

    #[test]
    fn a_draft_with_files_becomes_a_multipart_message() {
        let dir = std::env::temp_dir().join(format!("files-mail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("отчёт.pdf");
        std::fs::write(&file, b"%PDF-1.4 hello").unwrap();
        let draft = Draft { to: "bob@example.com; eve@example.com".into(), subject: "Report".into(), body: "See attached".into(), attachments: vec![file], ..Default::default() };
        let raw = build(&account(), &draft).unwrap().formatted();
        let parsed = parse_message(&raw).unwrap();
        assert_eq!(parsed.subject, "Report");
        assert_eq!(parsed.from, "Ann Lee <ann@gmail.com>");
        assert_eq!(parsed.to, "bob@example.com, eve@example.com");
        assert_eq!(parsed.body, "See attached");
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].name, "отчёт.pdf");
        assert_eq!(parsed.attachments[0].bytes, b"%PDF-1.4 hello");

        let saved = save_attachment(&parsed.attachments[0], &dir).unwrap();
        let again = save_attachment(&parsed.attachments[0], &dir).unwrap();
        assert_ne!(saved, again, "an existing file is never overwritten");
        std::fs::remove_dir_all(&dir).unwrap();

        let empty = Draft { to: " ".into(), ..Default::default() };
        assert!(build(&account(), &empty).is_err(), "no recipient");
    }

    #[test]
    fn headers_decode_and_attachment_names_stay_in_the_folder() {
        let summary = parse_summary(b"From: =?UTF-8?B?0JjQstCw0L0=?= <ivan@ya.ru>\r\nSubject: =?UTF-8?Q?=D0=9F=D1=80=D0=B8=D0=B2=D0=B5=D1=82?=\r\nDate: Sat, 31 Oct 2026 10:00:00 +0000\r\n\r\n");
        assert_eq!(summary.from, "Иван");
        assert_eq!(summary.subject, "Привет");
        assert!(summary.date.is_some());
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(safe_name(".."), "attachment");
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    struct TestHost;

    impl files_sdk::Host for TestHost {
        fn config_dir(&self) -> Option<PathBuf> {
            Some(std::env::temp_dir().join(format!("files-ext-mail-{}", std::process::id())))
        }

        fn protection(&self, _: &Path, _: &[String], _: &Path) -> Option<String> {
            None
        }

        fn delete(&self, _: &[&Path], _: bool) -> Result<(), (usize, io::Error)> {
            Ok(())
        }
    }

    #[test]
    fn search_reads_filters_and_the_chained_account() {
        let search = Search::parse("email:Vitalicset@gmail.com:to:someone@qwe.com invoice from:ann is:unread");
        assert_eq!(search.account.as_deref(), Some("vitalicset@gmail.com"));
        assert_eq!(search.criteria, ["TEXT \"invoice\"", "UNSEEN"]);
        assert_eq!(search.addresses, [(Address::To, "someone@qwe.com".into()), (Address::From, "ann".into())]);
        assert_eq!(Search::parse("subject:\"x").criteria, ["SUBJECT \"\\\"x\""], "quotes are escaped");
        assert!(Search::parse("").criteria.is_empty());
    }

    #[test]
    fn address_filters_match_part_of_an_address() {
        let summary = parse_summary(
            b"From: Vitalicset <Vitalicset@gmail.com>\r\nTo: Other <other@vitalicset.net>\r\nCc: copy@gmail.com, vitalicset+tag@gmail.com\r\n\r\n",
        );
        assert_eq!(summary.address, "Vitalicset@gmail.com");
        assert_eq!(summary.to, "other@vitalicset.net");
        assert_eq!(summary.cc, "copy@gmail.com, vitalicset+tag@gmail.com");
        assert!(Search::parse("from:vitalicset").matches(&summary));
        assert!(Search::parse("to:VITALICSET").matches(&summary));
        assert!(Search::parse("cc:vitalicset").matches(&summary));
        assert!(!Search::parse("to:missing").matches(&summary));
        let combined = Search::parse("to:vitalicset is:unread");
        assert_eq!(combined.criteria, ["UNSEEN"], "only the non-address filters are sent to IMAP");
        assert!(combined.matches(&summary));
    }

    #[test]
    fn html_mail_keeps_its_html_and_plain_mail_has_none() {
        let raw = b"From: Ann <ann@x.com>\r\nSubject: News\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHello\r\n--b\r\nContent-Type: text/html\r\n\r\n<p><b>Hello</b> <a href=\"https://x.com\">x</a></p>\r\n--b--\r\n";
        let message = parse_message(raw).unwrap();
        assert_eq!(message.body, "Hello");
        assert!(message.html.as_deref().is_some_and(|html| html.contains("<b>Hello</b>")));
        let plain = parse_message(b"From: ann@x.com\r\nSubject: Hi\r\n\r\nJust text\r\n").unwrap();
        assert_eq!(plain.html, None);
        assert_eq!(parse_summary(b"From: Ann Lee <ann@x.com>\r\n\r\n").address, "ann@x.com");
    }

    #[test]
    fn special_folders_are_found_by_flag_or_name() {
        use imap_proto::NameAttribute as A;
        assert_eq!(Special::of("INBOX", &[]), Some(Special::Inbox));
        assert_eq!(Special::of("[Gmail]/Bin", &[A::Trash]), Some(Special::Trash));
        assert_eq!(Special::of("Отправленные", &[]), Some(Special::Sent));
        assert_eq!(Special::of("Projects", &[]), None);
    }

    #[test]
    fn bcc_recipients_get_mail_but_stay_hidden() {
        let account = Account::for_provider("ann@gmail.com", "", Provider::Gmail).unwrap();
        let draft = Draft { to: "bob@x.com".into(), cc: "carol@x.com".into(), bcc: "secret@x.com".into(), subject: "Hi".into(), ..Default::default() };
        let message = build(&account, &draft).unwrap();
        let recipients: Vec<String> = message.envelope().to().iter().map(ToString::to_string).collect();
        assert_eq!(recipients, ["bob@x.com", "carol@x.com", "secret@x.com"]);
        let text = String::from_utf8(message.formatted()).unwrap();
        assert!(text.contains("Cc: carol@x.com"), "{text}");
        assert!(!text.contains("secret@x.com"), "Bcc leaked into the headers");
        assert_eq!(parse_message(text.as_bytes()).unwrap().cc, "carol@x.com");
    }

    #[test]
    fn scheduled_mail_waits_keeps_its_files_and_can_be_cancelled() {
        files_sdk::set_host(Box::new(TestHost));
        let dir = host().config_dir().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plan.txt");
        std::fs::write(&file, "v1").unwrap();
        let account = Account::for_provider("ann@gmail.com", "", Provider::Gmail).unwrap();
        let draft = Draft { to: "bob@x.com".into(), subject: "Later".into(), attachments: vec![file.clone()], ..Default::default() };
        let queued = schedule(&account, &draft, 2_000_000_000).unwrap();
        std::fs::write(&file, "v2 changed after scheduling").unwrap();
        let copy = &queued.draft.attachments[0];
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "v1", "the file is kept as it was");
        assert_eq!(scheduled().len(), 1);
        // Not due yet: nothing is sent.
        assert!(send_due(1_000_000_000).is_empty());
        // Due, but the account is not in mail.toml: it stays queued with why.
        let results = send_due(2_000_000_001);
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
        assert!(scheduled()[0].error.as_deref().is_some_and(|error| error.contains("removed")));
        cancel(queued.id).unwrap();
        assert!(scheduled().is_empty());
        assert!(!copy.exists(), "its copied files go too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod network {
    use super::*;

    /// Real servers: run with `cargo test -p files-ext-mail -- --ignored`.
    #[test]
    #[ignore]
    fn gmail_and_yandex_greet_over_tls() {
        for provider in [Provider::Gmail, Provider::Yandex] {
            let (imap, _) = provider.servers().unwrap();
            let mut client = imap::Client::new(connect_tls(&imap).unwrap());
            client.read_greeting().unwrap_or_else(|error| panic!("{}: {error}", imap.host));
            let wrong = client.login("nobody@example.com", "wrong");
            assert!(wrong.is_err(), "{} accepted a wrong password", imap.host);
        }
    }
}


#[cfg(test)]
mod smtp_network {
    use super::*;

    /// Real servers: SMTP reachable over TLS by name, even where IPv6 is not.
    #[test]
    #[ignore]
    fn smtp_servers_answer_over_tls() {
        use lettre::transport::smtp::client::{Tls as SmtpTls, TlsParameters};
        for provider in [Provider::Gmail, Provider::Yandex] {
            let (_, smtp) = provider.servers().unwrap();
            let ip = reachable_ip(&smtp).unwrap();
            let transport = lettre::SmtpTransport::builder_dangerous(ip.to_string())
                .port(smtp.port)
                .tls(SmtpTls::Wrapper(TlsParameters::new(smtp.host.clone()).unwrap()))
                .build();
            assert!(transport.test_connection().unwrap(), "{} did not answer", smtp.host);
        }
    }
}
