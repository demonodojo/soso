//! Árbol de cajas del documento gráfico.

use alloc::string::String;
use alloc::vec::Vec;

use crate::computed::ComputedStyle;
use crate::html::{GfxForm, GfxFormField, GfxImage, Link};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoxChild {
    Text(String),
    Link { idx: usize },
    Image { idx: usize },
    FormInput {
        form_idx: usize,
        field_idx: usize,
    },
    FormSubmit {
        form_idx: usize,
        label: String,
    },
    Element(usize),
}

#[derive(Clone, Debug)]
pub struct BoxNode {
    pub tag: String,
    pub class: String,
    pub id: String,
    pub inline_css: String,
    pub style: ComputedStyle,
    pub children: Vec<BoxChild>,
}

#[derive(Clone, Debug)]
pub struct BoxDocument {
    pub page_title: Option<String>,
    pub page_background: u32,
    pub root: usize,
    pub nodes: Vec<BoxNode>,
    pub links: Vec<Link>,
    pub images: Vec<GfxImage>,
    pub forms: Vec<GfxForm>,
    pub form_fields: Vec<GfxFormField>,
}

impl BoxDocument {
    pub fn node(&self, idx: usize) -> Option<&BoxNode> {
        self.nodes.get(idx)
    }

    pub fn node_mut(&mut self, idx: usize) -> Option<&mut BoxNode> {
        self.nodes.get_mut(idx)
    }
}
