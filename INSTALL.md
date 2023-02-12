# `INSTALL`

## Requirements

### Fedora

```bash
sudo dnf install just libusb1-devel dbus-devel bpftrace libbpf-tools bcc-tools
# Probably not needed
sudo dnf install bcc-lua bcc-devel
```

Then determine the path to `execsnoop` as such:

```bash
$ rpm -ql bcc-tools | grep -i execsnoop
/usr/share/bcc/tools/doc/execsnoop_example.txt
/usr/share/bcc/tools/execsnoop
/usr/share/man/man8/bcc-execsnoop.8.gz
```

As it can be seen from the previous output, it is not `/usr/sbin/execsnoop-bpfcc` but rather it is  `/usr/share/bcc/tools/execsnoop`. This requires us to set `EXECSNOOP_PATH` to `/usr/share/bcc/tools/execsnoop` before we build.

## Build and Install

```bash
env EXECSNOOP_PATH=/usr/share/bcc/tools/execsnoop just
sudo just install
sudo systemctl daemon-reload
sudo systemctl enable --now com.system76.Scheduler
sudo systemctl start --now com.system76.Scheduler
sudo systemctl status com.system76.Scheduler
journalctl --unit com.system76.Scheduler --follow
```
