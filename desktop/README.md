# Desktop build (planned)

The desktop build will be a Tauri shell for Linux, macOS and Windows around the
same frontend (`frontend/`) and the same Rust core (`core/`), running the core
natively instead of as WebAssembly. It adds real filesystem access, the OS
keychain for provider keys, long renders and batch work.

Nothing is built here yet: the first usable build is browser-first
(see `docs/decisions.md`). Because the core is deterministic across targets,
a project opened in either build renders bit-identically.
