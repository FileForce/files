# Changelog

Every release takes its notes from its section here (`## <version>`): the
release workflow copies it to the GitHub release, which the website's
downloads page and the app's "Check for Updates" show. Write the next
version's notes under "Unreleased", then rename it when you tag.

## 0.1.18

- Windows programs (.exe, .dll…) show their own icon, as in Explorer — in the grid, the Pro list and the preview, on every system — and the preview tells what they are: product, description, company, versions, copyright, original name; built for x64, x86 or ARM64, a windowed or console app or a library, .NET, signed (Authenticode), build date. Programs (.exe, .msi, .app, .AppImage) are their own kind, Application.
- Release builds: appimagetool comes from a pinned version with a mirror (GitHub served 504s for its usual file, which failed the Linux builds of 0.1.16 and 0.1.17).

## 0.1.17

- Big folders are much lighter: the Pro list and the Expressive grid build only the rows on screen (and a few around), not every file. A redraw of a 5,000-file folder went from ~595,000 allocations and ~390 MB allocated to ~7,000 and ~4 MB; scrolling and selecting got as light.
- Sorting compares names without making lowercase copies, and the file list is built once per change instead of several times a frame.
- Help → Report a Bug… (also the bug button beside the Settings gear, and About): describe the problem and it becomes an issue on GitHub — no GitHub account needed. Version and system are added; the last Console events only when you tick the box. If the website cannot take it, the issue opens filled in, in the browser.
- Release builds: the AppImage tools are downloaded with retries before the build, so a flaky download no longer fails the Linux release.

## 0.1.16

- Where Mail and Music open (a tab or their own window) moved from General to each extension's page in Settings → Extensions; Music can open in a window too. A choice made before for Mail is kept.
- Text in Settings — labels and explanations — can be selected and copied.
- Settings open in a tab too: "Open in tab" in the Settings header (unsaved edits come along), or middle-click the gear. With a Settings tab open, ⌘, goes to it.
- Settings → Appearance → Theme, apart from the color scheme (light, dark, system): Files' own colors, **Omarchy** (Tokyo Night, Tokyo Night Day in light, with Omarchy's terminal look: a monospace font, square corners, no shadows, visible box lines) and **Catppuccin** (Mocha in dark, Latte in light; the accent picks mauve, blue or peach).
- Settings → Appearance → Icons: colorful (a color per kind), monochrome, or all in the accent color.
- Windows: "Show in folder" from a browser (and other apps) selects the file that just arrived instead of only opening its folder; the folder verb is refreshed at each start so it follows the app after updates.

## 0.1.15

- macOS: a click on the Dock icon brings Files back when its window was closed (a new window opens) or minimized; folders sent from the system or a second start open a window again too.
- Preview shows everything a file says about itself. Photos: size, megapixels and shape, format, color space, orientation; camera, lens, software, date taken, author; aperture, shutter, ISO, focal length, exposure, flash, white balance; GPS location. Video and audio (through the bundled VLC): duration, overall bitrate, resolution; per stream codec, frame rate, channels, sample rate, bitrate, language; tags (title, artist, album, director, season…).
- Archives: the preview tells the format and how it is compressed: method and level (Deflate, gzip level, RAR Fastest…Best, Store), size before and after with the saving, file and folder counts, encryption, solid RAR, recovery record, comment.
- ISO disc images (.iso) open and extract like other archives (ISO 9660 with Joliet long names); the preview shows the label, disc size, bootable, created, publisher and the program that made it.
- Disk images mount from the context menu (Mount): ISO and IMG everywhere, DMG on macOS (hdiutil), VHD/VHDX on Windows (Mount-DiskImage, a drive letter), Linux through udisksctl without administrator rights. The image opens in a new tab and shows in CONNECTED, where Eject unmounts it (on Linux the loop device is freed too).
- Windows: CONNECTED lists the other drives (USB sticks, DVDs, mounted images, network drives).
- SDK: `ArchiveProvider::info` lets archive extensions describe an archive for the preview (`ArchiveInfo`).

## 0.1.14

- One Files at a time (Settings → General → Windows, on by default): opening a folder or starting Files again uses the running one, which opens the folder in a new tab and comes to the front. The second start talks to it over 127.0.0.1 with a random token only your user can read.
- Windows: Settings → Default apps can make Files open folders and drives instead of Explorer (double-click on the desktop and in Explorer, downloads, other apps), through your user's registry without administrator rights. "This PC", Control Panel and other shell folders go back to Explorer; Restore Explorer and uninstalling Files undo it.

## 0.1.13

- macOS: Settings → Default apps can make Files open folders instead of Finder: "Show in Finder" from other apps, downloads and `open .` open in Files, each in a new tab (the desktop, the Dock and Spotlight stay Finder's). Restore Finder undoes it.
- macOS: with that on, `open <folder>` in a terminal opens Files too: a small `open` script goes first in PATH (`~/.zshrc`, `~/.bash_profile`, between marked lines). Files, apps, packages and flags go to `/usr/bin/open` as before, and once Files is deleted folders open in Finder again. Restore Finder removes the script and the lines.

## 0.1.12

- Windows: Files has its own AppUserModelID (`dev.fileforce.files`), set by the app and on the Start menu shortcut, so its notifications show Files' name and icon and its windows group under one taskbar button.
- Linux: Files writes its desktop entry and icon to your user folders (menus, notifications and "Open with" know it), and Settings → Default apps can make it the default file manager (folders from the desktop, browsers' "Show in folder", other apps).
- Started with a folder or file (`file-force ~/Music`, `file://` addresses), Files opens it.
- System notifications: notifications also reach macOS Notification Center, the Windows action center and Linux desktop notifications. Settings → Sound → System notifications: never, when Files is in the background (default) or always; all, warnings and errors, or errors only; and a test button.
- Settings → Storage: a ring chart of what Files keeps on disk (thumbnails, extensions' caches, forks, the search index, sounds), each kind with its size and a Clear button, plus Clear all. Forks are shown but never deleted from here.
- SDK: `files_sdk::storage` gives each extension its own cache folder (`cache_dir`, `cache_file`, `usage`, `clear`, `touch`). Settings → Storage lists every extension's cache with its size and a Clear button, and the cache limit covers it.
- Music and SSH keep their caches through `files_sdk::storage`: Music's tracks and covers (moved over from the old temp folder at the first start) and files opened from SSH servers show as their own rows under Extensions.
- Cache limit on a slider (1 GB by default; 256 MB to 20 GB or none): past it, files used least recently are deleted first.
- Music covers are stored scaled down (96 and 480 px) instead of full size, so long libraries no longer fill memory with multi-megabyte pictures.
- Updates: Install & Relaunch installs the verified update and starts the
  new version: Windows runs the installer (with its progress bar),
  macOS replaces the app from the disk image, Linux replaces the AppImage.
  If that cannot be done, the installer opens as before.
- Notifications appear at the bottom right, above everything (dialogs
  too), newest at the bottom.
- Extensions SDK: `files_sdk::audio` plays a notification chime, a sound
  from a file or the web, or tracks in the app's music player (Now
  Playing, media keys), with pause / resume / next / previous / stop. It
  needs the new Sound permission, which users can revoke.

## 0.1.11

### Sound

- Notifications chime: a different sound for info, success, warning and
  error, at the volume you set (once, however many windows are open).
- New Settings → Sound: notification sounds (on/off, Soft / Bright / Low,
  volume, try each kind); a speaker test for headphones, 2.1, 3.1, 5.1 and
  7.1 (a tone in each speaker, a low one for the subwoofer, or all in
  turn); and the equalizer, shared with Music and song previews.

### Mail

- A Details panel (the ⓘ button, or the inspector when Mail is a tab)
  shows who sent a message and how: From, Reply-To, Return-Path, the
  sender's IP and the route through servers, the mail program, SPF / DKIM /
  DMARC checks, Message-ID, the MIME type of each part, attachments, size
  and every header, all selectable.
- Quick Look on a saved message (.eml) shows it with the same details.

### Music

- Yandex Music tracks start at once, streamed while they download (a grey
  bar on the seek slider shows how much has arrived); the next track of
  the queue is downloaded ahead, and played tracks stay cached.
- The inspector button opens a track panel on the right of the Music tab:
  a big cover, artist, album, length, where it comes from, and Like, Go to
  Artist, Download or Show in Files.

### Tabs

- Middle-click a tab to close it, like a browser.

## 0.1.10

### Tabs

- Middle-click a folder (in any view, or in the sidebar) to open it in a
  new tab, like a browser. Folders' right-click menu has Open in New Tab.

### Disks and servers

- CONNECTED in the sidebar: disks, USB drives and mounted disk images can
  be ejected (⏏ on hover, or right-click → Eject), and have Open, Open in
  New Tab, Copy Path and Properties.
- SSH servers have a right-click menu: Connect / Disconnect, Open in
  Terminal, Copy Address and Forget Server.

### Developer tools

- Console → Resources shows the app's memory (as Activity Monitor counts
  it on macOS, the resident set on Windows and Linux) and CPU over the last
  two minutes, and what the app keeps: file entries, the search index,
  mail and music in memory, open media players, and the caches on disk.

### Extensions SDK

- Sidebar rows can carry their own right-click menu
  (`SidebarItem::menu`, `SidebarMenuItem`).

## 0.1.9

### Music (new extension, experimental)

A Music tab and a MUSIC entry in the sidebar.

- **Your library**: the audio files in your music folders (Settings →
  Extensions → Music), with tags, covers, search and a queue. Right-click
  an audio file → Play in Music (its folder becomes the queue) or Add to
  Queue.
- **Spotify**: sign in in the browser; Liked Songs, playlists, search.
  The music plays in the Spotify app (Premium) and Files controls it, as
  Spotify requires.
- **Yandex Music** (unofficial API): sign in with a token; «Мне нравится»,
  playlists, search, and «Моя волна», the personal radio that plays
  endlessly and learns from plays and skips. Tracks play in Files.
- **Right-click a track**: Play Next, Add to Queue, Show in Files; for
  Spotify and Yandex Music also Like, Remove from Liked, Add to Playlist,
  Go to Artist and Artist Radio; Yandex Music also Dislike and Download
  (an MP3 with tags, into Downloads). Spotify offers the artist's top
  tracks instead of radio, which it closed to apps.
- **Sound**: volume and a 10-band equalizer with presets (Rock, Pop,
  Classical…), kept for every track and for previews.
- **Media keys**: the keyboard's play/next keys, headphone buttons and the
  system's Now Playing (macOS Control Center, the Windows media flyout,
  MPRIS on Linux) control Music, with the title and cover.
- The THIS COMPUTER, SPOTIFY and YANDEX MUSIC groups fold and unfold.

### Previews

- Quick Look (Space) on a song plays it, with its cover and tags.

### Search

- `music:` in global search (⌘K) finds tracks in the library, Spotify and
  Yandex Music, and plays the one you pick.
- `ext:` and `kind:` match the start of a name: `ext:web` finds .webp
  and .webm, `kind:vid` finds videos.

### Tabs

- Drag a tab to move it (pinned tabs stay in front).
- The tab menu shows its shortcuts: Close ⌘W, Close Other Tabs ⌥⌘W; the
  rest can be set in Settings → Hotkeys. Its items can be colored in
  Settings → Appearance.

### Fixes

- Memory: lists and grids show small copies of pictures instead of
  decoding every photo full size, and a folder's pictures are freed when
  you leave it. Long sessions in photo folders no longer grow to gigabytes.
- Opening a file from search scrolls to it, also in folders with hundreds
  of items.
- The Music track list stays smooth with thousands of tracks, and rows
  span the whole list.

## 0.1.8

- Power saving (Settings → General): Automatic, On or Off. While it is on,
  background refreshes slow down and animations stop.
- Power saving: Automatic can wait for the battery to drop to a level you
  pick (Settings → General, 5–100%; 100% = as soon as on battery). The
  charge is read on macOS, Windows and Linux and shown under the slider.
- Global search (Cmd-K): `email:` searches the mail of every account.
- Developer console: every changed setting, switch and slider is listed
  (`power_saving: "automatic" → "on"`), along with view, inspector and
  power saving changes; secrets are masked.
- Windows: the app icon is built into file-force.exe, so the window, the
  taskbar, Explorer and the Start menu shortcut keep it after updates.

## 0.1.7

- Videos play in Quick Look (Space): play/pause, seek, time and mute, on
  macOS, Windows and Linux. libVLC (LGPL) ships inside the macOS app and
  the Windows installer and the Linux AppImage (only what previews need,
  about 27 MB on macOS), so nothing has to be installed.
- New Mail extension (experimental): several accounts (Google, Yandex or
  any IMAP/SMTP server) under MAIL in the sidebar; sign-in with an app
  password kept in the system keychain.
  - Opens as a tab (or a window: Settings → General → Extension windows).
  - Special folders (Inbox, Starred, Drafts, Sent, Archive, Spam, Trash) are
    found from the server, with their own icons; pick their colors or hide
    them in Settings → Extensions → Mail.
  - Fast: one connection per account, messages kept in memory and the
    newest read ahead.
  - Links in messages open in the browser; long lines wrap.
  - Write with Cc and Bcc; Send later (in an hour, tomorrow, Monday or any
    time), listed under Scheduled until it goes out.
  - Search mail: `from:`, `to:`, `subject:`, `is:unread`, and
    `email:you@gmail.com:to:someone@x.com` to pick the account.
  - Right-click a file → Send as Attachment… (several selected at once).
  - Sending tries ports 465 and 587 over IPv4 first, and says so plainly
    when a network blocks outgoing mail.
  - Messages show with their HTML styles (or every line of plain text);
    subject, sender and text can be selected and copied.
  - Right-click a message: Reply, Mark as Read/Unread, Star, Copy Sender
    Address, Move to Spam, Delete (to Trash; for good inside Trash).
- Global search (Cmd-K) searches the folder set in Settings → General →
  Global search folder (the home folder by default); Change… in the search
  window picks another folder just this once, "Always search here" keeps it.
  While it searches it says so instead of "No matching files".
- Global search (Cmd-K) ranks in the background after a short pause, so
  filters like `git:` no longer freeze typing.
- An extension's page in Settings lists its search filters (Git: `git:`),
  each on or off for the folder search and for global search.
- Tabs: right-click for Close, Close Tabs to the Left / Right, Close Other
  Tabs and Pin Tab; pinned tabs stay at the left and survive bulk closes.

## 0.1.6

- Search filters from extensions. Git adds `git:`: `git:feature` finds
  items in repositories on a matching branch, `git:modified`,
  `git:untracked`, `git:staged`, `git:conflicted` and `git:clean` by status.
- Windows: the title bar buttons (global search, new window, inspector)
  work again, so a closed inspector can be reopened.
- Windows: no console window flashes up when checking for or downloading
  updates, or when any other program runs.

## 0.1.5

- Quick Look (Space) on ZIP, RAR, TAR and GZ archives lists what is inside.
- Windows: forks of folders on a ReFS volume (Dev Drive) are kept on that
  volume in `.file-force-forks`, so Windows 11 24H2+ makes them as block
  clones: instant and taking no extra space. NTFS still makes a full copy.

## 0.1.4

- RAR archives: preview, browse and extract on every system (no `unrar`
  needed).
- PDF previews on Windows and Linux (Linux needs poppler-utils).
- Windows: the context menu opens instantly, programs no longer flash a
  console window, "Choose app" in Default apps opens a file dialog, and
  Terminal offers Windows Terminal, PowerShell and Command Prompt.
- Terminal lists only the apps installed on this computer.
- An empty folder says "This folder is empty" instead of "No matching files".
- Accessibility settings say "System: on/off" instead of "macOS".

## 0.1.3

- "Files X is available" notification with Install (signed download),
  Later (asks again at the next check) and Skip this release.
- Settings → General → Check now: look for an update any time, with
  Download & Install and the release notes right there.

- "Download & Install" in Check for Updates: every release is signed in CI
  and Files checks the signature before opening the installer.

## 0.1.2

- After an update, a notification says what version you now have, with a
  button that opens its release notes.
- Notifications can carry an action button (for extensions too).
- Windows: no console window opens next to Files.
- The disk card at the bottom of the sidebar is a carousel of every disk
  (C:, D:, USB drives, network shares): arrows, dots, and two-finger swipes.
  Clicking a disk opens it.

## 0.1.1

- The sidebar shows your account name and "Local workspace" instead of the
  demo labels.
- The title bar reads "file FORCE".

## 0.1.0

The first release of Files.

- Fast file explorer: list, cards and dual-pane views, tabs, Quick Look
  (folders too), global search with filters (`kind:`, `size:`, `modified:`,
  `tag:`, `mode:`).
- Extensions: Git, SSH (with Tailscale sign-in and Keychain passwords), AI
  agents (Claude Code, Codex, OpenCode), archives, copy-on-write forks,
  terminal.
- Accessibility: reduce motion, high contrast, text contrast, text size,
  VoiceOver labels.
- Developer console, notifications, permissions per extension.
