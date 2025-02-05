use async_stream::stream;
use futures::{Stream, StreamExt};
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    hash::Hash,
};

use kube::{
    runtime::{
        reflector::{Lookup, ObjectRef},
        watcher,
    },
    Resource,
};

pub enum ResourceChange<T: Resource + Lookup> {
    Ready,
    Apply(T),
    Delete(ObjectRef<T>),
}

pub fn resource_changes<K, W>(
    stream: W,
) -> impl Stream<Item = Result<ResourceChange<K>, watcher::Error>>
where
    K: Lookup + Resource + Clone,
    <K as Lookup>::DynamicType: Eq + Hash + Clone + Default,
    W: Stream<Item = watcher::Result<watcher::Event<K>>>,
{
    let mut stream = Box::pin(stream);

    let mut objects = HashMap::new();
    let mut init_present = HashSet::new();

    stream! {
        while let Some(event) = stream.next().await {
            match event {
                Ok(watcher::Event::Init) => {
                    init_present = HashSet::new();
                }
                Ok(watcher::Event::InitApply(r)) => {
                    let key = make_key(&r);
                    let ver = r.resource_version().unwrap().into_owned();

                    // We track which elements are present in the init
                    // procedure, so that we can correctly emit remove
                    // events after init is done.
                    init_present.insert(key.clone());

                    match objects.entry(key) {
                        Entry::Occupied(mut occupied) => {
                            if occupied.get() != &ver {
                                occupied.insert(ver);
                                yield Ok(ResourceChange::Apply(r));
                            }
                            // Else resource has not changed, no need ot emit.
                        },
                        Entry::Vacant(vacant) => {
                            vacant.insert(ver);
                            yield Ok(ResourceChange::Apply(r));
                        },
                    }
                }
                Ok(watcher::Event::InitDone) => {
                    let mut removed = Vec::new();
                    objects.retain(|k, _v| {
                        if init_present.contains(k) {
                            true
                        } else {
                            removed.push(k.clone());
                            false
                        }
                    });

                    // New hashset to dealloc
                    init_present = HashSet::new();

                    for r in removed.drain(..) {
                        yield Ok(ResourceChange::Delete(r));
                    }

                    yield Ok(ResourceChange::Ready);
                }
                Ok(watcher::Event::Apply(r)) => {
                    let key = make_key(&r);
                    let ver = r.resource_version().unwrap().into_owned();

                    match objects.entry(key) {
                        Entry::Occupied(mut occupied) => {
                            if occupied.get() != &ver {
                                occupied.insert(ver);
                                yield Ok(ResourceChange::Apply(r));
                            }
                            // Else resource has not changed, no need ot emit.
                        },
                        Entry::Vacant(vacant) => {
                            vacant.insert(ver);
                            yield Ok(ResourceChange::Apply(r));
                        },
                    }
                }
                Ok(watcher::Event::Delete(r)) => {
                    let key = make_key(&r);
                    objects.remove(&key);
                    yield Ok(ResourceChange::Delete(key));
                }
                Err(err) => {
                    //log::error!("got error in service CRD watcher: {:?}, retrying..", err);
                    yield Err(err);
                }
            }
        }
    }
}

fn make_key<K: Lookup>(resource: &K) -> ObjectRef<K>
where
    K::DynamicType: Default,
{
    let dyn_type = K::DynamicType::default();
    resource.to_object_ref(dyn_type)
}
