# Contributing to Freenpipe

Thank you for considering a contribution. Freenpipe is an MIT fork of screenpipe at commit `892199f`. It stays compatible with screenpipe's data, API, pipes and MCP configurations, and builds its own features beyond that. That origin puts a few rules on every contribution, so read the first section before you write code.

## Clean-room rules (read first)

The rules are in [CLAUDE.md](CLAUDE.md), section "Provenance and clean-room rules". In short:

- Never consult post-relicense upstream material: screenpipe source, commits, diffs, pull requests, issues, docs or changelogs dated after 2026-06-10, the upstream `main` branch, or anything under upstream's `ee/` directory. Do not clone, fetch or browse them. The only exception is for behaviour authors, under the public-source rules in [SOURCING.md](docs/clean-room/SOURCING.md).
- Never examine, run or extract anything from a screenpipe binary, installer or bundled frontend. Binary analysis was frozen on 2026-10-29 (owner decision 8).
- Implement only from re-sourced items in [docs/clean-room/specs/](docs/clean-room/specs/README.md). Never read `docs/clean-room/legacy-specs/`. Redacting identifiers and writing behaviour are separate jobs with their own rules, described in [SOURCING.md](docs/clean-room/SOURCING.md); whoever does one of them for an item does not implement it.
- Add a provenance record for your work package: copy [docs/clean-room/provenance/TEMPLATE.md](docs/clean-room/provenance/TEMPLATE.md) to `docs/clean-room/provenance/<WP>.md`. CI checks for it (`bun test scripts/ci/clean-room-guard.test.ts`).
- Write original code and prose. Do not paste decompiled code, extracted frontend code, or extracted UI text and prompts.
- Interoperability identifiers may match upstream exactly: HTTP routes, JSON fields, SQL names, CLI flags, settings keys, environment variables, and file and directory layout.
- No phoning home: never send data to the upstream vendor's services or embed their keys. Telemetry stays off. Cloud features get a local or bring-your-own-key equivalent.

Where to look:

- [docs/clean-room/README.md](docs/clean-room/README.md): the method and the index of subsystem specs.
- [docs/clean-room/PLAN.md](docs/clean-room/PLAN.md): the work packages, their dependencies, owned files and acceptance checks.
- [docs/clean-room/DECISIONS.md](docs/clean-room/DECISIONS.md): owner decisions that override the plan.
- [docs/clean-room/STATUS.md](docs/clean-room/STATUS.md): which work packages are merged, claimed or ready.

`tools/clean-room/verbatim_gate.py` checks documents and source for prose that also occurs in the analysed binary. Run it on files you add or rewrite:

```bash
python3 -I tools/clean-room/verbatim_gate.py path/to/file.md
```

New source files carry the Freenpipe file header described in CLAUDE.md. When you edit an existing upstream file, leave its original header alone; it is attribution.

Before product or UI decisions, read [VISION.md](VISION.md) and [DESIGN.md](DESIGN.md).

## Building

Windows (x86_64-pc-windows-msvc) is the primary platform and must build and run. The macOS and Linux `cfg` paths must keep compiling, but this fork does not exercise them.

### Windows

Follow [docs/dev/windows-build.md](docs/dev/windows-build.md). It covers the toolchain (Rust pinned by `rust-toolchain.toml`, Visual Studio 2022 Build Tools, LLVM, CMake, bun), the native sidecars, the engine and desktop app builds, and dev isolation.

### macOS (not exercised in this fork)

1. Install dependencies:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   brew install pkg-config ffmpeg jq cmake wget git-lfs
   ```
   Install Xcode (the command line tools alone are not enough) and initialise it:
   ```bash
   sudo xcodebuild -license
   xcodebuild -runFirstLaunch
   ```

2. Install bun:
   ```bash
   curl -fsSL https://bun.sh/install | bash
   ```

3. Clone and build:
   ```bash
   git clone https://github.com/normiecore/Freenpipe
   cd Freenpipe
   cargo build --release --features metal,apple-intelligence
   ./target/release/screenpipe
   ```

4. Build the desktop app:
   ```bash
   cd apps/screenpipe-app-tauri
   bun install
   bun tauri build --features metal,apple-intelligence
   ```

### Linux (not exercised in this fork)

1. Install dependencies:
   ```bash
   sudo apt-get install -y g++ ffmpeg tesseract-ocr cmake libavformat-dev libavfilter-dev libavdevice-dev libssl-dev libtesseract-dev libxdo-dev libsdl2-dev libclang-dev libxtst-dev libpipewire-0.3-dev
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   curl -fsSL https://bun.sh/install | bash
   ```
   For AMD GPU acceleration, also install `libvulkan-dev glslc`.

2. Clone and build:
   ```bash
   git clone https://github.com/normiecore/Freenpipe
   cd Freenpipe
   cargo build --release
   ./target/release/screenpipe
   ```

3. Build the desktop app:
   ```bash
   sudo apt-get install -y libayatana-appindicator3-1 libayatana-appindicator3-dev librsvg2-dev libwebkit2gtk-4.1-dev
   cd apps/screenpipe-app-tauri
   bun install
   bun tauri build
   ```

### Docker

A third-party desktop environment image that includes the base project is described in [this Dockerfile](https://github.com/sabrehagen/desktop-environment/blob/730a3134362927f8965589f6322b4554e0a5e388/docker/Dockerfile#L403).

## Dev isolation

Freenpipe uses screenpipe's default data directory (`~/.screenpipe`) and ports (3030 for the API, 11435 for the app's control server). A development build must never use them, because the machine may also run an installed screenpipe or your own everyday Freenpipe.

Load the isolation variables in every dev shell:

```bash
source scripts/dev/dev-env.sh
```

This sets a separate data directory, port 3035 and a dev API key. To run an engine by hand with the same idea:

```bash
./target/release/screenpipe record --port 3035 --data-dir /tmp/sp
```

Use a throwaway data directory, especially when you test new database migrations. Details are in [docs/dev/windows-build.md](docs/dev/windows-build.md), section "Dev isolation".

## How to contribute

Questions, bug reports and proposals go to [GitHub issues](https://github.com/normiecore/Freenpipe/issues).

### Reporting bugs

- Use a clear, descriptive title.
- Describe the exact steps that reproduce the problem.
- Include your operating system, the commit you built from, and the relevant part of the log files in your data directory.

### Suggesting enhancements

- Use a clear, descriptive title.
- Describe the change step by step and explain who it helps.
- If it relates to a work package in [PLAN.md](docs/clean-room/PLAN.md), name the work package.

### Pull requests

- Fill in the pull request template.
- Do not put issue numbers in the title.
- Include screenshots or screen recordings for UI changes.
- Follow the Rust style guide and end every file with a newline.
- Keep a pull request to one work package or one fix.

## Style guides

### Commit messages

- Use the present tense and the imperative mood ("add feature", "move cursor to").
- Keep the first line to 72 characters or less.
- Reference issues and pull requests after the first line.
- Write a body that explains why, because the history is read to build changelogs.

### Rust

Rust code follows the [Rust style guide](https://github.com/rust-lang/rust/tree/4f2f477fded0a47b21ed3f6aeddeafa5db8bf518/src/doc/style-guide/src) and the standard [Cargo package layout](https://doc.rust-lang.org/cargo/guide/project-layout.html).

## Principles

- Build what people use, and make it work well.
- Concurrency: prefer channels to mutexes and locks.
- Simplicity: avoid premature optimisation. Write code that is easy to read and fast to run. Prefer less code, fewer files, fewer dependencies.
- Production quality: this software runs all day on people's machines.
- Focus: avoid feature creep. Build on the core functionality.
- Measure: if you cannot measure it, you cannot improve it.

## Issue and pull request labels

- `bug`: something does not work.
- `enhancement`: a feature request.
- `documentation`: documentation issues and changes.
- `good first issue`: suitable for newcomers.

## Running tests

Before you open a pull request, run the tests for what you changed:

```bash
cargo test     # Rust
bun test       # JavaScript and TypeScript
```

For the desktop app's TypeScript bindings, from `apps/screenpipe-app-tauri/`:

```bash
bun run bindings:check      # fails if lib/utils/tauri.ts drifted from the Rust surface
bun run bindings:generate   # regenerate after adding or changing commands
```

Read [TESTING.md](TESTING.md) before changing window management, the tray, monitors or audio.

## Other notes

### Debugging a GitHub Action

Open an SSH session into the runner:

```yaml
- name: Setup tmate session # HACK
  if: matrix.platform == 'windows-latest'
  uses: mxschmitt/action-tmate@v3
```

To run workflows locally, use [act](https://github.com/nektos/act).

### Debugging memory errors

With tokio-console:

```bash
# terminal 1
RUST_LOG="tokio=debug,runtime=debug" RUSTFLAGS="--cfg tokio_unstable" cargo run --bin screenpipe --features debug-console
# terminal 2
cargo install tokio-console
tokio-console
```

With sanitizers:

```bash
RUSTFLAGS="-Z sanitizer=address" cargo run --bin screenpipe
# or
RUSTFLAGS="-Z sanitizer=leak" cargo run --bin screenpipe
```

For leak tracking on macOS:

```bash
cargo install cargo-instruments
# track leaks with a 10 minute time limit
cargo instruments -t Leaks --bin screenpipe --features metal --time-limit 600000 --open
```

Then open the file in `target/release/instruments` in Xcode (Open Developer Tool, Instruments).

### Profiling CPU

To find where the engine spends CPU time, capture a sampling profile and a CPU and memory time series of the running process. The recipes below give comparable output on Windows and macOS.

**Windows** (samply, install once with `cargo install samply`):

```powershell
$sp = (Get-Process screenpipe-app,screenpipe -ErrorAction SilentlyContinue |
       Sort-Object WorkingSet64 -Descending | Select-Object -First 1).Id

# 1) cpu/mem time series, 10s interval for 10min
"ts,pcpu,ws_mb,priv_mb" | Out-File $env:TEMP\sp-cpu.csv
1..60 | ForEach-Object {
  $p1=Get-Process -Id $sp; $c1=$p1.TotalProcessorTime.TotalSeconds
  Start-Sleep 1
  $p2=Get-Process -Id $sp; $c2=$p2.TotalProcessorTime.TotalSeconds
  $pcpu=[math]::Round(($c2-$c1)*100,1)
  "$(Get-Date -Format HH:mm:ss),$pcpu,$([math]::Round($p2.WorkingSet64/1MB)),$([math]::Round($p2.PrivateMemorySize64/1MB))" |
    Add-Content $env:TEMP\sp-cpu.csv
  Start-Sleep 9
}

# 2) sampling profile, 10min, view at https://profiler.firefox.com
samply record --save-only -o $env:TEMP\sp.json.gz --duration 600 --pid $sp
```

If samply is unavailable, use `wpr -start CPU -filemode; Start-Sleep 600; wpr -stop sp.etl` and open the trace in [WPA](https://learn.microsoft.com/windows-hardware/test/wpt/windows-performance-analyzer) or PerfView.

**macOS** (built in, no install):

```bash
PID=$(pgrep -x screenpipe-app || pgrep -x screenpipe | head -1)

# 1) cpu/mem time series, 10s interval for 10min
( echo "ts,pcpu,pmem,rss_mb"
  for i in $(seq 1 60); do
    read pcpu pmem rss < <(ps -p $PID -o pcpu=,pmem=,rss=)
    echo "$(date +%H:%M:%S),$pcpu,$pmem,$((rss/1024))"
    sleep 10
  done ) > /tmp/sp-cpu.csv

# 2) sampling profile, 10min at 1ms (call tree, symbolicated)
sample $PID 600 -file /tmp/sp-sample.txt
```

Aggregate the hot leaf functions:

```bash
awk '/Call graph/,/Binary Images/' /tmp/sp-sample.txt \
  | grep '(in ' | sed -E 's/^[[:space:]+!|:]*//; s/  \(in .*$//' \
  | grep -E "^[0-9]+ (screenpipe|AXUIElement|cidre|onnxruntime|sqlite)" \
  | awk '{n=$1+0; $1=""; sub(/^ +/,"",$0); if(n>mx[$0])mx[$0]=n}
         END{for(k in mx) printf "%8d  %s\n", mx[k], k}' \
  | sort -rn | head -30
```

Attach both files (`sp-cpu.csv` and `sp.json.gz` or `sp-sample.txt`) to the issue when you report a CPU regression.

### Benchmarks

```bash
cargo bench
```

### Creating a migration

```bash
cargo install sqlx-cli
sqlx migrate add <migration_name>
```

Schema names are interoperability identifiers: a migration must keep Freenpipe able to read data written by screenpipe. The schema identifiers are the `interop` items of `db.md` in [docs/clean-room/specs/](docs/clean-room/specs/README.md).

### Fixing migration errors

If you see an error such as `migration XXXXXXXXXX was previously applied but is missing`, remove that migration from the database you are developing against. Point the commands at your dev data directory, not at a data directory you care about:

```bash
DB=/tmp/sp/db.sqlite

# remove a specific migration
sqlite3 "$DB" "DELETE FROM _sqlx_migrations WHERE version = XXXXXXXXXX;"

# list applied migrations
sqlite3 "$DB" "SELECT * FROM _sqlx_migrations;"

# last resort: back up the database, then reset the migrations table
cp "$DB" "$DB.backup"
sqlite3 "$DB" "DROP TABLE _sqlx_migrations;"
```

### Ubuntu VM on Azure with display and audio

```bash
# Set variables
RG_NAME="my-avd-rgg"
LOCATION="westus2"
VM_NAME="ubuntu-avd"
IMAGE="Canonical:0001-com-ubuntu-server-jammy:22_04-lts-gen2:latest"
VM_SIZE="Standard_D2s_v3"

# Create resource group
az group create --name $RG_NAME --location $LOCATION

# Create VM
az vm create \
  --resource-group $RG_NAME \
  --name $VM_NAME \
  --image $IMAGE \
  --admin-username azureuser \
  --generate-ssh-keys \
  --size $VM_SIZE

# Enable RDP
az vm open-port --port 3389 --resource-group $RG_NAME --name $VM_NAME

# Install xrdp, audio, and desktop environment
az vm run-command invoke \
  --resource-group $RG_NAME \
  --name $VM_NAME \
  --command-id RunShellScript \
  --scripts "
    sudo apt update && sudo apt install -y xrdp ubuntu-desktop pulseaudio
    sudo systemctl enable xrdp
    sudo adduser xrdp ssl-cert
    echo 'startxfce4' | sudo tee /etc/xrdp/startwm.sh
    sudo systemctl restart xrdp
    sudo ufw allow 3389/tcp
  "

# Enable audio redirection
az vm run-command invoke \
  --resource-group $RG_NAME \
  --name $VM_NAME \
  --command-id RunShellScript \
  --scripts "
    echo 'load-module module-native-protocol-tcp auth-anonymous=1' | sudo tee -a /etc/pulse/default.pa
    sudo systemctl restart pulseaudio
  "

# Get IP address
IP=$(az vm list-ip-addresses --resource-group $RG_NAME --name $VM_NAME --output table | grep -oE "\b([0-9]{1,3}\.){3}[0-9]{1,3}\b" | head -1)

# Connect with Microsoft Remote Desktop using this IP

# SSH into the VM
ssh azureuser@$IP

# Forward the RDP port to your machine
ssh -L 13389:localhost:3389 azureuser@$IP

# Change the password
az vm user update \
  --resource-group $RG_NAME \
  --name $VM_NAME \
  --username azureuser \
  --password <new-password>
```

You can then develop on Linux, or run the engine in the VM and record your local session over RDP. Configure Microsoft Remote Desktop to forward audio.

### Regenerating openapi.yaml

Run the engine (on a dev port), then fetch the generated spec:

```bash
curl -o openapi.yaml http://localhost:3035/openapi.yaml
```

The documentation copy lives at `docs/mintlify/docs-mintlify-mig-tmp/openapi.yaml`. The generated output usually needs manual fixes; validate it before you commit:

```bash
bunx @mintlify/scraping@latest openapi-file docs/mintlify/docs-mintlify-mig-tmp/openapi.yaml -o /tmp
node docs/mintlify/validate-docs.mjs
```
