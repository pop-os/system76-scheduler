pub fn set_priority(process: u32, priority: i32) {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, process, priority);
    }
}
