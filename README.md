# borbatop

A native Linux task manager for the terminal, made for Arch and Arch-based distributions.

One screen for processes, systemd services and pacman. It reads `/proc`, `/sys` and cgroup v2 directly: no daemon, no GUI, no async runtime. One Rust binary.

## What it shows

| Tab | Content |
|---|---|
| Processes | CPU, memory, threads, owner, systemd unit, owning package, full command line. Sort, filter, tree view, signals. |
| Services | systemd units with failures first, and the journal of the selected unit. |
| Pacman | Pending upgrades, orphans, running versus installed kernel, leftover `.pacnew` files, processes using replaced libraries. |
| Network · Disk | Throughput per interface and disk, fill level of each filesystem. |

The top of the screen always shows per-core CPU, memory, swap, pressure stall (PSI), load, uptime and temperature.

## Install

From source (Rust stable):

```bash
cargo install --path crates/borbatop
```

Arch packaging drafts live in `packaging/arch` (release) and `packaging/arch/borbatop-git` (development version). Both were built and inspected locally with `makepkg`, but `url` is still a placeholder: set it to the real repository before publishing anywhere.

## Use

```bash
borbatop                      # open the screen
borbatop --tree               # start with processes as a tree
borbatop --interval-ms 500    # sample twice per second
borbatop --theme ansi         # use the terminal's own 16 colors
borbatop completions zsh      # print a completion script (bash, zsh, fish, ...)
borbatop man > borbatop.1     # print the manual page
```

### Keys

| Key | Action |
|---|---|
| `↑` `↓` `j` `k` | move |
| `1` – `4`, `Tab` | switch tab |
| `s` / `r` | next sort column / reverse order |
| `/` | filter by command, user, unit or package |
| `t` | tree view |
| `p` | show or hide the package column |
| `K` | send a signal (SIGTERM, SIGHUP, SIGKILL) after confirmation |
| `l` | journal of the selected service |
| `R` | rebuild the Pacman report |
| `Space` | pause |
| `q`, `Ctrl+C` | quit |

### Configuration

`~/.config/borbatop/config.toml` (or `$XDG_CONFIG_HOME/borbatop/config.toml`); see `packaging/config.example.toml`. Every key is optional, unknown keys are rejected, and command-line flags win over the file.

## Requirements

- Linux with cgroup v2 and `pidfd` support (kernel 5.3 or newer).
- A terminal; truecolor is recommended, `--theme ansi` works without it.
- Optional: `systemd` for the Services tab, `pacman` for the Pacman tab and package names.

## Privacy and safety

- Command lines can hold secrets. They are shown on screen only, and hidden from `Debug` output so they cannot leak into logs.
- Text that comes from other processes, the journal or the filesystem is stripped of control characters before drawing, so it cannot inject terminal escape sequences.
- A signal is sent only to the process you picked: the target is pinned with a pidfd and its start time is checked, so a reused PID is never hit.

## Development

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo bench -p borbatop-core            # parsers
cargo bench -p borbatop-sources         # full machine read
```

Architecture notes for contributors are in [CONTEXT.md](CONTEXT.md).

## License

MIT OR Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
