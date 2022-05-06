# System76 Scheduler

Scheduling service which optimizes Linux's CPU scheduler and automatically assigns process priorities for improved desktop responsiveness. Low latency CPU scheduling will be activated automatically when on AC, and the default scheduling latencies set on battery. Processes are regularly sweeped and assigned process priorities based on configuration files. When combined with [pop-shell](https://github.com/pop-os/shell/), foreground processes and their sub-processes will be given higher process priority.

These changes result in a noticeable improvement in the experienced smoothness and performance of applications and games. The improved responsiveness of applications is most noticeable on older systems with budget hardware, whereas games will benefit from higher framerates and reduced jitter. This is because background applications and services will be given a smaller portion of leftover CPU budget after the active process has had the most time on the CPU.

## DBus

- Interface: `com.system76.Scheduler`
- Path: `/com/system76/Scheduler`

The `SetForeground(u32)` method can be called to change the active foreground process.

## Process Priority Config

RON configuration files are stored at the following locations:

- User-config: `/etc/system76-scheduler/assignments/`
- Distribution: `/usr/share/system76-scheduler/assignments/`

They define the default priorities for processes scanned. The syntax of `.ron` configuration files in these directories is as follows:

```rs
{
    (CPU_PRIORITY, IO_PRIORITY): [
        "exe_name1",
        "exe_name2"
    ],
    CPU_PRIORITY: [
        "exe_name3",
        "exe_name4"
    ],
    IO_PRIORITY: [
        "exe_name5"
    ]
}
```

Where:

- `CPU_PRIORITY` is a number between `-20` and `19`
- `IO_PRIORITY` is one of:
    - `Idle`
    - `Standard`
    - `BestEffort(PRIORITY_LEVEL)`
    - `Realtime(PRIORITY_LEVEL)`
- `PRIORITY_LEVEL` is a number between `0` and `7`


A real world example below:

```rs
{
    // Very high
    (-9, BestEffort(0)): [
        "easyeffects",
    ],
    // High priority
    (-5, BestEffort(4)): [
        "gnome-shell",
        "kwin",
        "Xorg"
    ],
    // Default
    0: [ "dbus", "dbus-daemon", "systemd"],
    // Absolute lowest priority
    (19, Idle): [
        "c++",
        "cargo",
        "clang",
        "cpp",
        "g++",
        "gcc",
        "lld",
        "make",
        "rustc",
    ]
}
```

## CPU Scheduler Latency Configurations

### Default

The default settings for CFS by the Linux kernel. Achieves a high level of throughput for CPU-bound tasks at the cost of increased latency for inputs. This setting is ideal for servers and laptops on battery, because low-latency scheduling sacrifices some energy efficiency for improved responsiveness.

```yaml
latency: 6ns
minimum_granularity: 0.75ms
wakeup_granularity: 1.0ms
bandwidth_size: 5us
```

### Responsive

Slightly reduces time given to CPU-bound tasks to give more time to other processes, particularly those awaiting and responding to user inputs. This can significantly improve desktop responsiveness for a slight penalty in throughput on CPU-bound tasks.

```yaml
latency: 4ns
minimum_granularity: 0.4ms
wakeup_granularity: 0.5ms
bandwidth_size: 3us
```

## License

Licensed under the [Mozilla Public License 2.0](https://choosealicense.com/licenses/mpl-2.0/). Permissions of this copyleft license are conditioned on making available source code of licensed files and modifications of those files under the same license (or in certain cases, one of the GNU licenses). Copyright and license notices must be preserved. Contributors provide an express grant of patent rights. However, a larger work using the licensed work may be distributed under different terms and without source code for files added in the larger work.

### Contribution

Any contribution intentionally submitted for inclusion in the work by you shall be licensed under the Mozilla Public License 2.0 (MPL-2.0). It is required to add a boilerplate copyright notice to the top of each file:

```rs
// Copyright {year} {person OR org} <{email}>
// SPDX-License-Identifier: MPL-2.0
```
