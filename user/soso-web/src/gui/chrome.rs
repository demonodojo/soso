//! Cromo: pestañas, barra de navegación y marcadores.

use alloc::string::String;
use alloc::vec::Vec;

use super::layout::{CHROME_HEIGHT, TAB_ROW_HEIGHT};

pub const BACK_W: f32 = 52.0;
pub const GO_W: f32 = 36.0;
pub const NEW_TAB_W: f32 = 28.0;
pub const CLOSE_TAB_W: f32 = 28.0;
pub const BM_LIST_W: f32 = 36.0;
pub const BM_STAR_W: f32 = 36.0;
pub const MAX_TABS: usize = 4;
/// URL de pestaña nueva; en runtime usar [`crate::rutas::demo_tab_url`].
pub const DEFAULT_TAB_URL: &str = "file:///var/paquetes/soso-web/share/ejemplo/web-prueba.html";

#[derive(Clone, Debug, Default)]
pub struct TabMeta {
    pub url_text: String,
    pub hist: Vec<String>,
    pub hist_pos: usize,
    pub error: Option<String>,
    pub scroll: f32,
}

impl TabMeta {
    pub fn new(url: String) -> Self {
        Self {
            url_text: url.clone(),
            hist: Vec::from([url]),
            hist_pos: 0,
            ..Self::default()
        }
    }

    pub fn can_back(&self) -> bool {
        self.hist_pos > 0
    }

    pub fn can_forward(&self) -> bool {
        self.hist_pos + 1 < self.hist.len()
    }

    pub fn go_back(&mut self) -> Option<String> {
        if !self.can_back() {
            return None;
        }
        self.hist_pos -= 1;
        let u = self.hist[self.hist_pos].clone();
        self.url_text = u.clone();
        self.error = None;
        Some(u)
    }

    pub fn go_forward(&mut self) -> Option<String> {
        if !self.can_forward() {
            return None;
        }
        self.hist_pos += 1;
        let u = self.hist[self.hist_pos].clone();
        self.url_text = u.clone();
        self.error = None;
        Some(u)
    }

    pub fn navigate(&mut self, url: String) {
        if self.hist_pos + 1 < self.hist.len() {
            self.hist.truncate(self.hist_pos + 1);
        }
        self.hist.push(url.clone());
        self.hist_pos = self.hist.len() - 1;
        self.url_text = url;
        self.error = None;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Chrome {
    pub tabs: Vec<TabMeta>,
    pub active: usize,
    pub url_focus: bool,
    pub field_focus: Option<usize>,
}

impl Chrome {
    pub fn new(initial: String) -> Self {
        Self {
            tabs: Vec::from([TabMeta::new(initial)]),
            active: 0,
            ..Self::default()
        }
    }

    pub fn active_tab(&self) -> &TabMeta {
        &self.tabs[self.active]
    }

    pub fn active_tab_mut(&mut self) -> &mut TabMeta {
        &mut self.tabs[self.active]
    }

    pub fn can_new_tab(&self) -> bool {
        self.tabs.len() < MAX_TABS
    }

    pub fn new_tab(&mut self) -> bool {
        if !self.can_new_tab() {
            return false;
        }
        self.tabs
            .push(TabMeta::new(crate::rutas::demo_tab_url()));
        self.active = self.tabs.len() - 1;
        self.url_focus = false;
        self.field_focus = None;
        true
    }

    pub fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.remove(self.active);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.url_focus = false;
        self.field_focus = None;
        true
    }

    pub fn select_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
            self.url_focus = false;
            self.field_focus = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeHit {
    NewTab,
    CloseTab,
    Tab(usize),
    Back,
    UrlField,
    Go,
    BookmarkStar,
    BookmarkList,
    None,
}

pub fn hit_chrome(mx: i32, my: i32, fb_w: f32) -> ChromeHit {
    let y = my as f32;
    let x = mx as f32;
    if y >= CHROME_HEIGHT {
        return ChromeHit::None;
    }
    if y < TAB_ROW_HEIGHT {
        if x < NEW_TAB_W {
            return ChromeHit::NewTab;
        }
        if x < NEW_TAB_W + CLOSE_TAB_W {
            return ChromeHit::CloseTab;
        }
        let tab_w = 72.0;
        let start = NEW_TAB_W + CLOSE_TAB_W;
        let mut i = 0usize;
        let mut tx = start;
        while tx + tab_w <= fb_w - BM_LIST_W - BM_STAR_W && i < MAX_TABS {
            if x >= tx && x < tx + tab_w {
                return ChromeHit::Tab(i);
            }
            tx += tab_w;
            i += 1;
        }
        if x >= fb_w - BM_LIST_W {
            return ChromeHit::BookmarkList;
        }
        if x >= fb_w - BM_LIST_W - BM_STAR_W {
            return ChromeHit::BookmarkStar;
        }
        return ChromeHit::None;
    }
    if x < BACK_W {
        ChromeHit::Back
    } else if x >= fb_w - GO_W {
        ChromeHit::Go
    } else if x >= BACK_W {
        ChromeHit::UrlField
    } else {
        ChromeHit::None
    }
}

pub fn tab_label(tab: &TabMeta) -> &str {
    let s = tab.url_text.as_str();
    if s.len() > 10 {
        &s[..10]
    } else {
        s
    }
}

pub fn url_bar_y() -> f32 {
    TAB_ROW_HEIGHT + 6.0
}
