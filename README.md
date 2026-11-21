# borbatop

A native Linux task manager for the terminal, made for Arch and Arch-based distributions.

Website: <https://af2b.github.io/borbatop/>

One screen for processes, systemd services, pacman, sockets, boot time, the system journal, GPUs and containers. It reads `/proc`, `/sys` and cgroup v2 directly: no daemon, no GUI, no async runtime. One Rust binary.

## What it shows

| Tab | Content |
|---|---|
| Processes | CPU, memory, threads, owner, systemd unit, owning package, full command line. Sort, filter, tree view, signals, renice (priority), CPU affinity. `Enter` opens everything known about the process: files, sockets, limits, memory. |
| Services | systemd units with failures first, the journal of the selected unit, start / stop / restart. |
| Pacman | Pending upgrades (fresh with `checkupdates`, otherwise from the local database), AUR upgrades for every `yay` or `paru` installed, orphans, running versus installed kernel, leftover `.pacnew` files, processes using replaced libraries. `u` chooses what to update. |
| Network · Disk | Throughput per interface and disk, fill level of each filesystem. |
| Connections | Every TCP and UDP socket with the process that owns it. |
| Boot | `systemd-analyze` summary and the slowest units. |
| Logs | Errors and warnings of the system journal since boot. |
| GPU | AMD, Intel and NVIDIA cards: load, video memory, temperature. |
| Containers | Podman and Docker containers, running or stopped, with the CPU and memory of their processes. Start, stop, restart, remove and read logs. |

The top of the screen always shows per-core CPU, memory, swap, pressure stall (PSI), load, uptime and temperature. When a reading goes over its threshold (CPU, memory, swap, pressure, temperature, failed units) the header lists it in red, whatever tab is open.

## Install

From source (Rust stable):

```bash
cargo install --path crates/borbatop
```

Arch packaging drafts live in `packaging/arch` (release) and `packaging/arch/borbatop-git` (development version). Both were built and inspected locally with `makepkg`; they are not published to the AUR yet.

## Use

```bash
borbatop                      # open the screen
borbatop --tree               # start with processes as a tree
borbatop --interval-ms 500    # sample twice per second
borbatop --theme ansi         # use the terminal's own 16 colors
borbatop --no-mouse           # leave the mouse to the terminal
borbatop completions zsh      # print a completion script (bash, zsh, fish, ...)
borbatop man > borbatop.1     # print the manual page
```

For scripts and later analysis:

```bash
borbatop snapshot                         # one sample as JSON on standard output
borbatop snapshot --with-commands         # ... including full command lines
borbatop record trace.jsonl -n 120        # 120 samples to a new file (JSON Lines)
borbatop replay trace.jsonl               # look at it on screen; Space pauses, ←/→ step
```

### Keys

| Key | Action |
|---|---|
| `↑` `↓` `j` `k` | move |
| `1` – `9`, `Tab` | switch tab |
| `Enter` | details of the selected process |
| `s` / `r` | next sort column / reverse order (the cursor goes back to the first row) |
| `/` | filter by command, user, unit or package |
| `t` | tree view |
| `p` | show or hide the package column |
| `K` | send a signal (SIGTERM, SIGHUP, SIGKILL) after confirmation |
| `n` / `c` | renice / choose the CPUs of the selected process |
| `a` | restart, stop or start the selected service; start, stop, restart or remove the selected container |
| `l` | journal of the selected service, or logs of the selected container |
| `u` | Pacman tab: choose what to update (repositories, AUR helpers) |
| `e` | export the tab being shown to a JSON file |
| `R` | rebuild the report of the current tab |
| `Space` | pause |
| `←` `→` | previous / next sample while replaying a recording |
| `q`, `Ctrl+C` | quit |

With the mouse: click a tab, a column title (sorts; click again to reverse) or a row; the wheel moves the selection.

### Priority (nice)

`n` sets the *nice* value of the selected process, from -20 to 19. It decides who goes first when processes compete for the CPU: a higher number makes the process more polite and yields to the others, 0 is normal, -20 is the most favored. It does not cap CPU use; an idle machine lets a nice-19 process use everything.

Typical use: give a heavy job (a build, a backup) `nice 10` or more so the rest of the machine stays responsive. Lowering priority is always allowed for your own processes; raising it (a negative value) needs root. The value is applied to every thread of the process, because Linux keeps it per thread.

### Exporting

`e` writes the tab you are looking at to `borbatop-<tab>-<YYYYMMDD-HHMMSS>.json` (UTC) in the directory where borbatop was started, keeping the filter and order you see. Command lines are never written. The file is readable by its owner only, and an existing file is never overwritten. For scripts, `borbatop snapshot` prints a whole sample instead.

### Updating packages

`u` on the Pacman tab lists what can be updated: the official repositories as **one** full upgrade (`sudo pacman -Syu`, never one package at a time, because a single-package upgrade leaves Arch half-upgraded), and for each installed AUR helper (`yay`, `paru`) either everything or a package choice. The command then runs on your real terminal, so `sudo` asks for the password there and borbatop never sees it; when it ends, press Enter to come back and the report refreshes.

### Configuration

`~/.config/borbatop/config.toml` (or `$XDG_CONFIG_HOME/borbatop/config.toml`); see `packaging/config.example.toml`. Every key is optional, unknown keys are rejected, and command-line flags win over the file. The `[alerts]` table sets the level at which each reading raises an alert.

## Requirements

- Linux with cgroup v2 and `pidfd` support (kernel 5.3 or newer).
- A terminal; truecolor is recommended, `--theme ansi` works without it.
- Optional: `systemd` for the Services and Boot tabs, `pacman` for the Pacman tab and package names, `pacman-contrib` (`checkupdates`) for a fresh upgrade list, `yay` or `paru` for AUR upgrades, `nvidia-smi` for NVIDIA readings, `podman` or `docker` for the Containers tab.
- Starting and stopping services needs privileges: `systemctl` runs without asking for a password, so an unprivileged attempt fails with the reason. Raising a process's priority needs root; lowering it never does.

## Privacy and safety

- Command lines can hold secrets. They are shown on screen only, and hidden from `Debug` output so they cannot leak into logs.
- Text that comes from other processes, the journal or the filesystem is stripped of control characters before drawing, so it cannot inject terminal escape sequences.
- A signal is sent only to the process you picked: the target is pinned with a pidfd and its start time is checked, so a reused PID is never hit. Renice and CPU affinity check the start time right before the change (the kernel offers no pidfd form of those calls).
- `snapshot` and `record` leave command lines out unless `--with-commands` is given. A recording is created readable by its owner only, and an existing file is never overwritten.
- While replaying a recording, actions that would change the machine are refused. Tabs that read the machine on demand (Pacman, Connections, Boot, Logs, GPU, Containers) show the current machine, not the recording.
- The details of a process (open files, sockets) and the journal are shown on screen only, and hidden from `Debug` output.
- Container names and package names are validated before any command is built, and commands are run without a shell. Container actions and updates are refused while replaying a recording.

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
