rootdir := ''
prefix := '/usr'
sysconfdir := '/etc'
root := rootdir + prefix
debug := '0'
vendor := '0'

target := if debug == '1' { 'debug' } else { 'release' }
vendor_args := if vendor == '1' { '--frozen --offline' } else { '' }
debug_args := if debug == '1' { '' } else { '--release' }
cargo_args := vendor_args + ' ' + debug_args

binary := 'system76-scheduler'
id := 'com.system76.Scheduler'

bindir := root + '/bin'
libdir := root + '/lib'
confdir := rootdir + sysconfdir

target_bin := bindir + '/' + binary

# Path to execsnoop binary.
execsnoop := '/usr/sbin/execsnoop-bpfcc'

# Compile pop-launcher
all: _extract_vendor
    env EXECSNOOP_PATH={{execsnoop}} cargo build {{cargo_args}}

# Remove Cargo build artifacts
clean:
    cargo clean

# Also remove .cargo and vendored dependencies
distclean:
    rm -rf .cargo vendor vendor.tar target

# Install everything
install:
    mkdir -p {{confdir}}/system76-scheduler/assignments \
        {{confdir}}/system76-scheduler/exceptions
    install -Dm0644 data/config.ron {{confdir}}/system76-scheduler/config.ron
    install -Dm0644 data/assignments.ron {{confdir}}/system76-scheduler/assignments/default.ron
    install -Dm0644 data/exceptions.ron {{confdir}}/system76-scheduler/exceptions/default.ron
    install -Dm0755 target/{{target}}/{{binary}} {{target_bin}}
    install -Dm0644 data/{{id}}.service {{libdir}}/systemd/system/{{id}}.service
    install -Dm0644 data/{{id}}.conf {{confdir}}/dbus-1/system.d/{{id}}.conf

# Uninstalls everything (requires same arguments as given to install)
uninstall:
    rm -rf {{confdir}}/system76-scheduler \
        {{confdir}}/dbus-1/system.d/{{id}}.conf \
        {{libdir}}/systemd/system/{{id}}.service \
        {{target_bin}}

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

# Extracts vendored dependencies if vendor=1
_extract_vendor:
    #!/usr/bin/env sh
    if test {{vendor}} = 1; then
        rm -rf vendor
        tar pxf vendor.tar
    fi