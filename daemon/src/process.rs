use crate::{service::OwnedPriority, utils::Buffer};
use concat_in_place::strcat;
use qcell::{LCell, LCellOwner};
use std::{
    collections::{
        hash_map::{DefaultHasher, Entry},
        HashMap, HashSet,
    },
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, Weak},
};

#[derive(Default)]
pub struct Process<'owner> {
    pub id: u32,
    pub parent_id: u32,
    pub name: String,
    pub cgroup: String,
    pub cmdline: String,
    pub forked_cmdline: String,
    pub forked_name: String,
    pub parent: Option<Weak<LCell<'owner, Process<'owner>>>>,
    pub assigned_priority: OwnedPriority,
}

impl<'owner> Hash for Process<'owner> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.id.hash(hasher);
        self.parent_id.hash(hasher);
    }
}

impl<'owner> Process<'owner> {
    pub fn ancestors<'a>(
        &self,
        owner: &'a LCellOwner<'owner>,
    ) -> impl Iterator<Item = Arc<LCell<'owner, Process<'owner>>>> + 'a {
        let mut parent_node = self.parent();
        std::iter::from_fn(move || {
            if let Some(parent) = parent_node.take() {
                parent_node = parent.ro(owner).parent();
                return Some(parent);
            }

            None
        })
    }

    pub fn hash_id(&self) -> u64 {
        let mut hasher = DefaultHasher::default();
        self.hash(&mut hasher);
        hasher.finish()
    }

    pub fn parent(&self) -> Option<Arc<LCell<'owner, Process<'owner>>>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }
}

#[derive(Default)]
pub struct Map<'owner> {
    pub map: HashMap<u64, Arc<LCell<'owner, Process<'owner>>>>,
    pub drain: HashSet<u64>,
}

impl<'owner> Map<'owner> {
    /// Removes processes that remain in the drain filter.
    pub fn drain_filter(&mut self) {
        for hash in self.drain.drain() {
            self.map.remove(&hash);
        }
    }

    /// This will be used to keep track of what processes were destroyed since the last refresh.
    pub fn drain_filter_prepare(&mut self) {
        self.drain.clear();
        for hash in self.map.keys() {
            self.drain.insert(*hash);
        }
    }

    pub fn get_pid(
        &self,
        token: &LCellOwner<'owner>,
        pid: u32,
    ) -> Option<&Arc<LCell<'owner, Process<'owner>>>> {
        self.map
            .values()
            .find(|&process| process.ro(token).id == pid)
    }

    pub fn insert(
        &mut self,
        owner: &mut LCellOwner<'owner>,
        process: Process<'owner>,
    ) -> Arc<LCell<'owner, Process<'owner>>> {
        match self.map.entry(process.hash_id()) {
            Entry::Occupied(entry) => {
                {
                    let entry = entry.get().rw(owner);
                    std::mem::swap(&mut entry.forked_cmdline, &mut entry.cmdline);
                    std::mem::swap(&mut entry.forked_name, &mut entry.name);
                    entry.name = process.name;
                    entry.cmdline = process.cmdline;
                }

                entry.get().clone()
            }
            Entry::Vacant(entry) => {
                let process = Arc::new(LCell::new(process));

                entry.insert(process.clone());
                process
            }
        }
    }

    /// Removes a process and its parents from consideration for process removal.
    pub fn retain_process_tree(&mut self, owner: &LCellOwner<'owner>, process: &Process<'owner>) {
        self.drain.remove(&process.hash_id());

        for parent in process.ancestors(owner) {
            let parent = parent.ro(owner);
            self.drain.remove(&parent.hash_id());
        }
    }
}

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

    Some(
        exe.as_os_str()
            .to_string_lossy()
            .split_whitespace()
            .next()
            .map(String::from)
            .unwrap_or_default(),
    )
}

pub fn exists(buffer: &mut Buffer, pid: u32) -> bool {
    buffer.path.clear();
    Path::new(strcat!(&mut buffer.path, "/proc/" buffer.itoa.format(pid) "/status")).exists()
}

pub fn name(cmdline: &str) -> &str {
    cmdline.rsplit('/').next().unwrap_or(cmdline)
}

pub fn parent_id(buffer: &mut Buffer, pid: u32) -> Option<u32> {
    buffer.path.clear();

    let path = strcat!(&mut buffer.path, "/proc/" buffer.itoa.format(pid) "/status");

    if let Some(value) = crate::utils::file_key(&mut buffer.file_raw, path, "PPid:") {
        return atoi::atoi::<u32>(value);
    }

    None
}
