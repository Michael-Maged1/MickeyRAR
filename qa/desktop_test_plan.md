# Required actual acceptance tests — not yet executed

## Build matrix
- Windows 10/11 x64: `cargo test`, `cargo run`, package and launch
- macOS Apple Silicon and x64: `cargo test`, `cargo run`, package and launch
- Linux x64 GNOME/KDE: `cargo test`, `cargo run`, package and launch

## Functional tests
- Create 7z from files with Arabic, spaces, long paths, and Unicode; reopen and compare SHA-256 before/after
- Test 1GB and 10GB sample files with repetitive, random, JPEG, MP4 and mixed content; measure compression ratio, RAM and elapsed time
- Test opening and extracting ZIP, RAR, 7z and damaged archives, including password-protected archives
- Verify operation on empty, locked and read-only destinations
- Verify extracting untrusted archives: paths with ../, absolute paths, symlinks, hardlinks, nested links, UNC paths, Windows device names, oversized decompression bombs
- Confirm no data loss when archive output coincides with selected source or output archive already exists
- Language: startup in English, switch Arabic, close/restart, inspect RTL and mixed-direction paths
- Responsiveness: controls remain accessible while operations run; cancellation and progress reflect real work
- A11y: keyboard navigation, focus, scaling at 125%, 150%, 200%, contrast and screen reader labels

**Release gate:** until every required feature passes real desktop tests, do not mark the app production-ready or claim that all buttons work.
