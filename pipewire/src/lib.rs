use pipewire as pw;
use pw::{
    node::{Node, NodeInfo},
    proxy::ProxyT,
    spa::ReadableDict,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
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
    Add(Process),
    Remove(u32),
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
                managed.insert(pw_id, process.id);
                func(ProcessEvent::Add(process));
            }
        }

        NodeEvent::Remove(pw_id) => {
            if let Some(id) = managed.remove(&pw_id) {
                func(ProcessEvent::Remove(id));
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
