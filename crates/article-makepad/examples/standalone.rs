//! A local-only host proving the editor needs neither Robrix nor OctoSense.
//! Pass --data-dir=<directory>; the example never connects to a server.
pub use makepad_widgets;
use makepad_widgets::*;
use article_core::{
    document::*, host::{ArticleHost, Capabilities, ConsentGrant, SessionAuthority},
    storage::LocalStore,
};
use article_makepad::{presentation, rich_input::ArticleRichInputWidgetRefExt};
use article_makepad::body_selection::{ArticleSelection, SelectionUpdate};
use article_core::editing::EditHistory;
use std::{path::{Path, PathBuf}, time::Duration};
app_main!(App);

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    let app = startup() do #(App::script_component(vm)) {
        ui: Root {
            main_window := Window {
                window.inner_size: vec2(840, 760)
                body +: {flow: Down padding: 24 spacing: 16
                    Label {text: "独立文章编辑器 · Standalone article editor" draw_text.text_style.font_size: 22}
                    Label {text: "Local profile · shared article-core + article-makepad"}
                    title := TextInput {width: Fill height: 46 empty_text: "文章标题 / Article title"}
                    View {width: Fill height: Fit spacing: 8
                        bold := Button {text: "Bold / 加粗" grab_key_focus: false}
                        italic := Button {text: "Italic / 斜体" grab_key_focus: false}
                        theme := Button {text: "Theme / 主题" grab_key_focus: false}
                        save := Button {text: "Save / 保存" grab_key_focus: false}
                        reload := Button {text: "Reload / 重新打开" grab_key_focus: false}
                    }
                    blocks := PortalList {width: Fill height: 260
                        Text := View {width: Fill height: Fit flow: Down
                            rich := ArticleRichInput {
                                width: Fill height: Fit is_multiline: true flow: Flow.Right{wrap: true}
                                padding: Inset{top: 8 bottom: 12 left: 8 right: 8}
                                draw_text.text_style: theme.font_regular{font_size: 14}
                                draw_bold.text_style: theme.font_bold{font_size: 14}
                                draw_italic.text_style: theme.font_italic{font_size: 14}
                                draw_bold_italic.text_style: theme.font_bold_italic{font_size: 14}
                            }
                        }
                    }
                    ScrollYView {width: Fill height: Fill
                        preview := Html {width: Fill height: Fit padding: 10 selectable: true
                            text_style_normal: theme.font_regular{font_size: 14}
                            text_style_bold: theme.font_bold{font_size: 14}
                            rmath := ArticleMath {} rdiagram := ArticleDiagram {}
                            rimage := ArticleImage {} remoji := ArticleEmoji {}
                            rcode := ArticleCode {} rcell := ArticleCell {}
                        }
                    }
                    status := Label {text: "Ready / 就绪"}
                }
            }
        }
    }
    app
}

struct LocalProfile { root: PathBuf, authority: SessionAuthority }
impl Default for LocalProfile {
    fn default() -> Self {
        let root = std::env::args().find_map(|a| a.strip_prefix("--data-dir=").map(PathBuf::from))
            .unwrap_or_else(|| std::env::temp_dir().join("article-standalone-example"));
        Self { root, authority: SessionAuthority::default() }
    }
}
impl ArticleHost for LocalProfile {
    fn active_account(&self) -> Option<String> { Some("local:writer".into()) }
    fn data_root(&self) -> &Path { &self.root }
    fn authority(&self) -> &SessionAuthority { &self.authority }
}
#[derive(Script, ScriptHook)]
struct App {
    #[live] ui: WidgetRef,
    #[rust] host: LocalProfile,
    #[rust] consent: Option<ConsentGrant>,
    #[rust] document: Document,
    #[rust] selected: usize,
    #[rust] body_selection: ArticleSelection,
    #[rust] history: EditHistory,
}
impl App {
    fn refresh(&self, cx: &mut Cx) {
        self.ui.text_input(cx, ids!(title)).set_text(cx, &self.document.title);
        let mut preview = self.ui.html(cx, ids!(preview));
        presentation::style_html(cx, preview.clone(), &self.document);
        let images=article_makepad::content::Images::default();
        let mut renderer=article_makepad::content::NativeRenderer{images:&images,size:if self.document.large_type {16.0}else{14.0},ink:self.document.theme.colors().1};
        let html=article_core::markdown_render::render(&self.document.markdown(),&mut renderer).iter().map(|b|article_makepad::content::native_html(&b.html)).collect::<String>();
        preview.set_text(cx, &html);
        self.ui.redraw(cx);
    }
    fn load(&mut self, cx: &mut Cx) {
        let store: LocalStore<'_, LocalProfile> = LocalStore::new(&self.host, self.consent.as_ref().unwrap());
        match store.load() {
            Ok(library) => {
                self.document = library.documents.into_iter().next().unwrap_or_else(||
                    Document::from_markdown("山野来信 · Independent host", "你好世界。Select words and apply **bold** formatting.\n\n## 共享组件\n\n草稿、字体和文章主题，无需登录任何服务。").unwrap());
                self.selected = 0;
                self.body_selection.reset();
                self.history.clear();
                self.ui.portal_list(cx, ids!(blocks)).set_first_id_and_scroll(0, 0.0);
                self.refresh(cx);
                self.ui.label(cx, ids!(status)).set_text(cx, "Loaded / 已打开");
            }
            Err(error) => self.ui.label(cx, ids!(status)).set_text(cx, &error),
        }
    }
}
impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.consent = Some(self.host.authority.issue("local:writer".into(), Capabilities::editor(), Duration::from_secs(3600)));
        self.load(cx);
    }
    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        if let Some(title) = self.ui.text_input(cx, ids!(title)).changed(actions) { self.document.title = title; }
        for (index, row) in self.ui.portal_list(cx, ids!(blocks)).items_with_actions(actions) {
            self.selected = index;
            let input = row.article_rich_input(cx, ids!(rich));
            if input.changed(actions).is_some() {
                self.history.checkpoint(&self.document);
                if let (Some(block), Some((text, marks))) = (self.document.blocks.get_mut(index), input.content()) {
                    block.text = text; block.marks = marks;
                }
            }
        }
        for (id, bold) in [(id!(bold), true), (id!(italic), false)] {
            if self.ui.button(cx, &[id]).clicked(actions) {
                if let Some(selection) = self.body_selection.selection {
                    let (start, _) = selection.ordered();
                    let flags = self.document.blocks[start.block].flags_at(start.byte);
                    self.history.checkpoint(&self.document);
                    for (index, block) in self.document.blocks.iter_mut().enumerate() {
                        if let Some(range) = selection.range(index, block.text.len()).filter(|r| !r.is_empty()) {
                            let _ = block.format(range, if bold { Some(!flags.0) } else { None },
                                if bold { None } else { Some(!flags.1) }, None);
                        }
                    }
                    continue;
                }
                let row = self.ui.portal_list(cx, ids!(blocks)).item(cx, self.selected, id!(Text));
                let input = row.article_rich_input(cx, ids!(rich));
                if input.toggle_format(cx, bold) {
                    if let (Some(block), Some((text, marks))) = (self.document.blocks.get_mut(self.selected), input.content()) {
                        block.text = text; block.marks = marks;
                    }
                }
            }
        }
        if self.ui.button(cx, ids!(theme)).clicked(actions) {
            let next = (Theme::ALL.iter().position(|t| *t == self.document.theme).unwrap_or(0) + 1) % Theme::ALL.len();
            self.document.theme = Theme::ALL[next];
        }
        if self.ui.button(cx, ids!(save)).clicked(actions) {
            let store: LocalStore<'_, LocalProfile> = LocalStore::new(&self.host, self.consent.as_ref().unwrap());
            let result = store.save_document(&self.document);
            self.ui.label(cx, ids!(status)).set_text(cx, result.as_ref().err().map(String::as_str).unwrap_or("Saved locally / 已本地保存"));
        }
        if self.ui.button(cx, ids!(reload)).clicked(actions) { self.load(cx); }
        self.refresh(cx);
    }
    fn handle_draw(&mut self, cx: &mut Cx, event: &DrawEvent) {
        let mut draw = CxDraw::new(cx, event);
        let mut cx = Cx2d::new(&mut draw);
        while let Some(widget) = self.ui.draw(&mut cx, &mut Scope::empty()).step() {
            if let Some(mut list) = widget.borrow_mut::<PortalList>() {
                list.set_item_range(&mut cx, 0, self.document.blocks.len());
                while let Some(index) = list.next_visible_item(&mut cx) {
                    if let Some(block) = self.document.blocks.get(index) {
                        let row = list.item(&mut cx, index, id!(Text));
                        let input = row.article_rich_input(&cx, ids!(rich));
                        presentation::style_input(&mut cx, input.clone(), &self.document, block);
                        input.set_empty_text(&mut cx, if presentation::show_body_placeholder(&self.document, index) {
                            "写下你的文章 / Write your article…".into()
                        } else { String::new() });
                        self.body_selection.apply_to_input(&mut cx, index, &input, &self.document);
                        row.draw_all(&mut cx, &mut Scope::empty());
                        self.body_selection.after_draw(&mut cx, index, &input);
                    }
                }
            }
        }
    }
}
impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        script_eval!(vm, {mod.theme = mod.themes.light});
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        article_makepad::apple_fonts::install(vm);
        makepad_widgets::widgets_mod(vm);
        article_makepad::script_mod(vm);
        self::script_mod(vm)
    }
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        let list = self.ui.portal_list(cx, ids!(blocks));
        match self.body_selection.handle_event(cx, event, &list, &mut self.document, &mut self.history) {
            SelectionUpdate::Changed => { self.refresh(cx); return; }
            SelectionUpdate::Handled => { return; }
            SelectionUpdate::Pass => {}
        }
        if !matches!(event, Event::Draw(_)) { self.ui.handle_event(cx, event, &mut Scope::empty()); }
        if matches!(event, Event::MouseDown(_) | Event::MouseUp(_)) {
            self.body_selection.after_event(cx, &list, &self.document);
        }
        self.match_event(cx, event);
    }
}
