// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::str::FromStr;

use kdl::{KdlDocument, KdlEntry, KdlNode, NodeKey};

pub fn fields(document: &KdlDocument) -> impl Iterator<Item = (&str, &KdlNode)> {
    document
        .nodes()
        .iter()
        .map(|node| (node.name().value(), node))
}

pub trait NodeExt {
    /// Check the value of the `enable` property if set.
    fn enabled(&self) -> Option<bool>;

    fn get_bool(&self, index: impl Into<NodeKey>) -> Option<bool>;

    fn get_string(&self, index: impl Into<NodeKey>) -> Option<&str>;

    fn get_u16(&self, index: impl Into<NodeKey>) -> Option<u16>;
}

impl NodeExt for KdlNode {
    fn enabled(&self) -> Option<bool> {
        self.get("enable")?.value().as_bool()
    }

    fn get_bool(&self, index: impl Into<NodeKey>) -> Option<bool> {
        self.get(index)?.value().as_bool()
    }

    fn get_string(&self, index: impl Into<NodeKey>) -> Option<&str> {
        self.get(index)?.value().as_string()
    }

    fn get_u16(&self, index: impl Into<NodeKey>) -> Option<u16> {
        u16::try_from(self.get(index)?.value().as_i64()?).ok()
    }
}

pub fn iter_properties(node: &KdlNode) -> impl Iterator<Item = (&str, &KdlEntry)> {
    node.entries()
        .iter()
        .filter_map(|entry| entry.name().map(|id| (id.value(), entry)))
}

pub trait EntryExt {
    fn as_i8(&self) -> Option<i8>;

    fn as_u8(&self) -> Option<u8>;

    fn parse_to<T: FromStr>(&self) -> Option<T>;
}

impl EntryExt for KdlEntry {
    fn as_i8(&self) -> Option<i8> {
        self.value().as_i64().and_then(|raw| i8::try_from(raw).ok())
    }

    fn as_u8(&self) -> Option<u8> {
        self.value().as_i64().and_then(|raw| u8::try_from(raw).ok())
    }

    fn parse_to<T: FromStr>(&self) -> Option<T> {
        self.value()
            .as_string()
            .and_then(|raw| raw.parse::<T>().ok())
    }
}
