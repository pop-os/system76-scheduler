use crate::utils::Buffer;
use concat_in_place::strcat;

pub fn cgroup(buffer: &mut Buffer, pid: u32) -> Option<&str> {
    buffer.path.clear();

    let path = strcat!(&mut buffer.path, "/proc/" buffer.itoa.format(pid) "/cgroup");

    let Ok(buffer) = crate::utils::read_into_string(&mut buffer.file, path) else {
        return None;
    };

    memchr::memchr(b':', buffer.as_bytes()).map(|pos| &buffer[pos + 2..buffer.len() - 1])
}

pub fn cmdline(buffer: &mut Buffer, pid: u32) -> Option<String> {
    buffer.path.clear();

    let path = strcat!(&mut buffer.path, "/proc/" buffer.itoa.format(pid) "/exe");

    let Ok(exe) = std::fs::read_link(path) else {
        return None;
    };

    Some(exe.as_os_str().to_string_lossy().to_string())
}

pub fn name(buffer: &mut Buffer, pid: u32) -> Option<&str> {
    buffer.path.clear();

    let path = strcat!(&mut buffer.path, "/proc/" buffer.itoa.format(pid) "/status");

    crate::utils::file_key(&mut buffer.file_raw, path, "Name:")
        .and_then(|name| std::str::from_utf8(name).ok())
}
