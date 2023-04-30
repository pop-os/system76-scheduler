binary := 'system76-scheduler'
id := 'com.system76.Scheduler'

rootdir := ''
prefix := '/usr'
sysconfdir := '/etc'

bindir := clean(rootdir / prefix) / 'bin'
libdir := clean(rootdir / prefix) / 'lib'
confdir := clean(rootdir / sysconfdir)

target-bin := bindir / binary

rustflags := env_var_or_default('RUSTFLAGS', '')

# Use the lld linker if it is available.
export RUSTFLAGS := if `which lld || true` != '' {
    rustflags + ' -C link-arg=-fuse-ld=lld -C link-arg=-Wl,--build-id=sha1 -Clink-arg=-Wl,--no-rosegment'
} else {
    rustflags
}

# Path to execsnoop binary.
execsnoop := `which execsnoop || which execsnoop-bpfcc`

[private]
default: build-release

# Remove Cargo build artifacts
clean:
    cargo clean

# Also remove .cargo and vendored dependencies
distclean:
    rm -rf .cargo vendor vendor.tar target

# Compile with debug profile
build-debug *args:
    env EXECSNOOP_PATH={{execsnoop}} cargo build {{args}}

# Compile with release profile
build-release *args: (build-debug '--release' args)

# Compile with a vendored tarball
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Check for errors and linter warnings
check *args:
    env EXECSNOOP_PATH={{execsnoop}} cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Runs a check with JSON message format for IDE integration
check-json: (check '--message-format=json')

# Install everything
install:
    mkdir -p {{confdir}}/system76-scheduler/process-scheduler
    install -Dm0644 data/config.kdl {{confdir}}/system76-scheduler/config.kdl
    install -Dm0644 data/pop_os.kdl {{confdir}}/system76-scheduler/process-scheduler/pop_os.kdl
    install -Dm0755 target/release/{{binary}} {{target-bin}}
    install -Dm0644 data/{{id}}.service {{libdir}}/systemd/system/{{id}}.service
    install -Dm0644 data/{{id}}.conf {{confdir}}/dbus-1/system.d/{{id}}.conf

# Uninstalls everything (requires same arguments as given to install)
uninstall:
    rm -rf {{confdir}}/system76-scheduler \
        {{confdir}}/dbus-1/system.d/{{id}}.conf \
        {{libdir}}/systemd/system/{{id}}.service \
        {{target-bin}}

# Vendor Cargo dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync bin/Cargo.toml \
        --sync plugins/Cargo.toml \
        --sync service/Cargo.toml \
        | head -n -1 > .cargo/config
    echo 'directory = "vendor"' >> .cargo/config
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
[private]
vendor-extract:
    rm -rf vendor
    tar pxf vendor.tar