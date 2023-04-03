use bstr::BStr;
use pipewire as pw;
use pw::{
    node::{Node, NodeInfo},
    proxy::ProxyT,
    spa::ReadableDict,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    io,
    os::unix::prelude::{AsRawFd, OwnedFd},
    rc::Rc,
    time::Duration,
};

#[derive(Debug)]
pub enum NodeEvent<'a> {
    Info(u32, &'a NodeInfo),
    Remove(u32),
}

#[derive(Debug)]
pub enum ProcessEvent {
    Add(u32),
    Remove(u32),
}

impl ProcessEvent {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut fields = BStr::new(bytes).split(|b| *b == b' ');

        let method = fields.next()?;
        let pid = atoi::atoi::<u32>(fields.next()?)?;

        match method {
            b"add" => Some(ProcessEvent::Add(pid)),
            b"rem" => Some(ProcessEvent::Remove(pid)),
            _ => None,
        }
    }

    /// # Errors
    ///
    /// - Failure to write bytes to writer
    pub fn to_bytes<W: std::io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let (method, pid) = match self {
            ProcessEvent::Add(pid) => (b"add", *pid),
            ProcessEvent::Remove(pid) => (b"rem", *pid),
        };

        writer.write_all(method)?;
        writer.write_all(b" ")?;
        writer.write_all(itoa::Buffer::new().format(pid).as_bytes())
    }
}

#[must_use]
#[derive(Copy, Clone, Debug)]
pub struct Process {
    pub id: u32,
}

impl Process {
    #[must_use]
    pub fn from_node(info: &NodeInfo) -> Option<Self> {
        let props = info.props()?;
        props.get("application.process.binary")?;

        Some(Process {
            id: props.get("application.process.id")?.parse::<u32>().ok()?,
        })
    }
}

/// Monitors the processes from a given ``PipeWire`` socket.
///
/// ``PipeWire`` sockets are found in `/run/user/{{UID}}/pipewire-0`.
pub fn processes_from_socket(socket: &OwnedFd, mut func: impl FnMut(ProcessEvent) + 'static) {
    let mut managed = BTreeMap::new();

    let _res = nodes_from_socket(socket, move |event| match event {
        NodeEvent::Info(pw_id, info) => {
            if let Some(process) = Process::from_node(info) {
                if managed.insert(pw_id, process.id).is_none() {
                    func(ProcessEvent::Add(process.id));
                }
            }
        }

        NodeEvent::Remove(pw_id) => {
            if let Some(pid) = managed.remove(&pw_id) {
                func(ProcessEvent::Remove(pid));
            }
        }
    });
}

/// Listens to information about nodes, passing that info into a callback.
///
/// # Errors
///
/// Errors if the pipewire connection fails
pub fn nodes_from_socket(
    socket: &OwnedFd,
    func: impl FnMut(NodeEvent) + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let main_loop = pw::MainLoop::new()?;
    let context = pw::Context::new(&main_loop)?;
    let core = context.connect_fd(socket.as_raw_fd(), None)?;

    let registry = Rc::new(core.get_registry()?);
    let registry_weak = Rc::downgrade(&registry);

    let nodes = Rc::new(RefCell::new(HashMap::new()));
    let func = Rc::new(RefCell::new(func));

    let remove_ids = Rc::new(RefCell::new(Vec::new()));

    let garbage_collector = main_loop.add_timer({
        let nodes = Rc::downgrade(&nodes);
        let remove_ids = Rc::downgrade(&remove_ids);
        move |_| {
            if let Some(nodes) = nodes.upgrade() {
                if let Some(remove_ids) = remove_ids.upgrade() {
                    for id in remove_ids.borrow_mut().drain(..) {
                        nodes.borrow_mut().remove(&id);
                    }
                }
            }
        }
    });

    let _res = garbage_collector
        .update_timer(Some(Duration::from_secs(60)), Some(Duration::from_secs(60)))
        .into_result();

    let _registry_listener = registry
        .add_listener_local()
        .global(move |obj| {
            let Some(registry) = registry_weak.upgrade() else {
                return;
            };

            if pw::types::ObjectType::Node == obj.type_ {
                let Ok(node): Result<Node, _> = registry.bind(obj) else {
                    return;
                };

                let proxy = node.upcast_ref();
                let id = proxy.id();

                let func_weak = Rc::downgrade(&func);

                let info_listener = node
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(func) = func_weak.upgrade() {
                            func.borrow_mut()(NodeEvent::Info(id, info));
                        }
                    })
                    .register();

                let func = Rc::downgrade(&func);
                let remove_ids = Rc::downgrade(&remove_ids);

                let remove_listener = proxy
                    .add_listener_local()
                    .removed(move || {
                        if let Some(remove_ids) = remove_ids.upgrade() {
                            remove_ids.borrow_mut().push(id);
                        }

                        if let Some(func) = func.upgrade() {
                            func.borrow_mut()(NodeEvent::Remove(id));
                        }
                    })
                    .register();

                nodes
                    .borrow_mut()
                    .insert(id, (node, info_listener, remove_listener));
            }
        })
        .register();

    main_loop.run();
    Ok(())
}
