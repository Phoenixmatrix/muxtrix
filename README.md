# Muxtrix

Muxtrix is a GPU-accelerated, cross-platform terminal workspace for running and
supervising many concurrent development sessions. It is written primarily in
Rust with [Iced](https://iced.rs/) for the application UI and Ghostty's VT
engine for terminal emulation.

> **Status: early, and vibe coded.** Muxtrix is pre-1.0 and was built almost
> entirely by prompting AI agents. No attempt was made to make the code good —
> it has not been designed, reviewed, or held to any quality bar, so please
> don't read it as an example of how to write Rust. Expect rough edges and
> breaking changes between releases.

## License

Muxtrix is available under the [MIT License](LICENSE).

Third-party attributions and the exact license texts for Ghostty, Herdr,
libghostty-rs, terminal theme sources, and embedded Unicode data are recorded
in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Release archives and the
Debian package include that file.
