use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Global, IntoElement, Render,
    SharedString, Subscription, Window, WindowBounds,
    component::{
        ActiveTheme, IconName, Sizable, TitleBar,
        button::{Button, ButtonVariants},
        h_flex,
        setting::{
            NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
        },
        v_flex,
    },
    div, img,
    prelude::*,
    px, size,
};

use crate::preferences::{Preferences, PreferencesData};

#[derive(Default)]
struct SettingsWindow(Option<AnyWindowHandle>);
impl Global for SettingsWindow {}

pub(crate) fn open(cx: &mut App) {
    if !cx.has_global::<SettingsWindow>() {
        cx.set_global(SettingsWindow::default());
    }
    if let Some(handle) = cx.global::<SettingsWindow>().0
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let mut options = crate::native_window_options("碧果设置", cx, size(px(900.), px(680.)));
    options.window_min_size = Some(size(px(720.), px(520.)));
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsPanel::new(window, cx))
    }) {
        Ok((handle, _)) => cx.global_mut::<SettingsWindow>().0 = Some(handle),
        Err(error) => tracing::error!(%error, "unable to open settings"),
    }
}

pub(crate) fn about(cx: &mut App) {
    let mut options = TitleBar::window_options();
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some("关于碧果".into());
    }
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        None,
        size(px(460.), px(390.)),
        cx,
    )));
    if let Err(error) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| AboutPanel)) {
        tracing::error!(%error, "unable to open About window");
    }
}

struct AboutPanel;
impl Render for AboutPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .refine_style(&crate::ui::font_style(
                &Preferences::shared(cx).read(cx).data,
            ))
            .child(TitleBar::new().child("关于碧果"))
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .p_6()
                    .child(img(crate::brand::icon(cx)).size(px(56.)))
                    .child(
                        div()
                            .text_size(px(25.))
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .child("碧果"),
                    )
                    .child(format!(
                        "版本 {} · 原生 Markdown 阅读器",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("清晰阅读，自在思考。"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("STIX Two Math：SIL OFL · Latin Modern Math：GUST"),
                    )
                    .child(
                        Button::new("project-license")
                            .small()
                            .ghost()
                            .label("查看开源许可证")
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/matheusmota-dev-br/mathra")
                            }),
                    ),
            )
    }
}

struct SettingsPanel {
    prefs: Entity<Preferences>,
    installed_fonts: Vec<(SharedString, SharedString)>,
    _subscription: Subscription,
}
impl SettingsPanel {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = Preferences::shared(cx);
        let subscription = cx.observe(&prefs, |_, _, cx| cx.notify());
        let installed_fonts = font_options(cx.text_system().all_font_names());
        Self {
            prefs,
            installed_fonts,
            _subscription: subscription,
        }
    }
    fn choice(
        &self,
        title: &str,
        options: Vec<(SharedString, SharedString)>,
        get: fn(&PreferencesData) -> &str,
        set: fn(&mut PreferencesData, String),
    ) -> SettingItem {
        let reader = self.prefs.clone();
        let writer = self.prefs.clone();
        SettingItem::new(
            title.to_string(),
            SettingField::scrollable_dropdown(
                options,
                move |cx| get(&reader.read(cx).data).to_string().into(),
                move |value, cx| {
                    writer.update(cx, |prefs, cx| {
                        prefs.change(|data| set(data, value.to_string()), cx)
                    })
                },
            ),
        )
    }
    fn number(
        &self,
        title: &str,
        min: f64,
        max: f64,
        step: f64,
        get: fn(&PreferencesData) -> f64,
        set: fn(&mut PreferencesData, f64),
    ) -> SettingItem {
        let reader = self.prefs.clone();
        let writer = self.prefs.clone();
        SettingItem::new(
            title.to_string(),
            SettingField::number_input(
                NumberFieldOptions { min, max, step },
                move |cx| get(&reader.read(cx).data),
                move |value, cx| {
                    writer.update(cx, |prefs, cx| prefs.change(|data| set(data, value), cx))
                },
            ),
        )
    }
}

fn font_options(mut names: Vec<String>) -> Vec<(SharedString, SharedString)> {
    names.sort();
    names.dedup();
    let mut options = vec![
        (pecan_mermaid::SYSTEM_UI_FONT.into(), "系统默认字体".into()),
        (
            pecan_mermaid::SYSTEM_MONO_FONT.into(),
            "系统默认等宽字体".into(),
        ),
    ];
    options.extend(
        names
            .into_iter()
            .filter(|name| !name.starts_with('.') && !name.trim().is_empty())
            .map(|name| (name.clone().into(), name.into())),
    );
    options
}

impl Render for SettingsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = &self.prefs.read(cx).data;
        let ui_style = crate::ui::font_style(prefs);
        let ui_size = prefs.ui_size;
        let reader = self.prefs.clone();
        let writer = self.prefs.clone();
        let reset = self.prefs.clone();
        let close_reader = self.prefs.clone();
        let close_writer = self.prefs.clone();
        let pages = [
            SettingPage::new("通用").icon(IconName::Settings).group(SettingGroup::new().title("文件与设置").items([
                SettingItem::new("外部修改后自动重新加载", SettingField::switch(move |cx|reader.read(cx).data.auto_reload, move |v,cx|writer.update(cx,|p,cx|p.change(|d|d.auto_reload=v,cx)))),
                SettingItem::new("关闭最后一个标签时退出应用", SettingField::switch(move |cx|close_reader.read(cx).data.quit_on_last_tab, move |v,cx|close_writer.update(cx,|p,cx|p.change(|d|d.quit_on_last_tab=v,cx)))),
                SettingItem::render(move |_,_,_| Button::new("reset-preferences").ghost().label("恢复默认设置").on_click({ let reset=reset.clone(); move |_,_,cx|reset.update(cx,|p,cx|p.change(|d|*d=PreferencesData::default(),cx)) })),
            ])),
            SettingPage::new("外观").icon(IconName::Sun).group(SettingGroup::new().title("界面").items([
                self.choice("主题", vec![("system".into(),"跟随系统".into()),("light".into(),"浅色".into()),("dark".into(),"深色".into())], |p| &p.theme, |p,v| p.theme=v),
                self.choice("界面中文字体", self.installed_fonts.clone(), |p| &p.ui_chinese_font, |p,v| p.ui_chinese_font=v),
                self.choice("界面英文字体", self.installed_fonts.clone(), |p| &p.ui_font, |p,v| p.ui_font=v).description("默认中文跟随系统，英文使用系统等宽字体；可分别选择系统安装的字体。"),
                self.number("界面字号",11.,20.,1.,|p|p.ui_size,|p,v|p.ui_size=v),
            ])),
            SettingPage::new("阅读").icon(IconName::BookOpen).group(SettingGroup::new().title("正文排版").items([
                self.choice("中文字体", self.installed_fonts.clone(), |p| &p.chinese_font, |p,v|p.chinese_font=v),
                self.choice("英文字体", self.installed_fonts.clone(), |p| &p.english_font, |p,v|p.english_font=v).description("默认中文跟随系统，英文使用系统等宽字体。中文作为回退；所选英文字体自带汉字时优先使用其字形。"),
                self.number("正文字号",12.,32.,1.,|p|p.font_size,|p,v|p.font_size=v).description("默认 16px，独立于代码块字号。"),
                self.number("行高",1.2,2.6,0.1,|p|p.line_height,|p,v|p.line_height=v),
                self.number("正文宽度",480.,1600.,40.,|p|p.reading_width,|p,v|p.reading_width=v).description("默认 960px；窗口较窄时自动收缩，图片跟随正文可用宽度。"),
            ])).group(SettingGroup::new().title("代码块").items([
                self.choice("代码字体", self.installed_fonts.clone(), |p| &p.code_font, |p,v|p.code_font=v).description("独立于正文字体，也用于行内 code 胶囊。"),
                self.number("代码块字号",10.,32.,1.,|p|p.code_size,|p,v|p.code_size=v).description("默认 14px；不自动换行，长行支持水平滚动。"),
                self.choice("缩进与 Tab 宽度", vec![("auto".into(),"自动识别".into()),("2".into(),"2 列".into()),("4".into(),"4 列".into()),("8".into(),"8 列".into())], |p| &p.code_tab_size, |p,v|p.code_tab_size=v).description("加载时按此宽度自动格式化支持的代码；块内可单独切换宽度。自动识别无法推断时使用 4 列。源 Markdown 不改写，未支持的语言保留原排版。"),
            ])),
            SettingPage::new("Mermaid").icon(gpui_kit::assets::IconName::Network).group(SettingGroup::new().title("图表字体").items([
                self.choice("中文字体", self.installed_fonts.clone(), |p| &p.mermaid_chinese_font, |p,v|p.mermaid_chinese_font=v),
                self.choice("英文字体", self.installed_fonts.clone(), |p| &p.mermaid_english_font, |p,v|p.mermaid_english_font=v).description("使用系统安装的字体；中英文分别排版，同时应用于正文图表和放大预览。"),
            ])),
            SettingPage::new("LaTeX").icon(gpui_kit::assets::IconName::Sigma).group(SettingGroup::new().title("LaTeX 排版").items([
                self.choice("LaTeX 字体", vec![("stix".into(),"STIX Two Math".into()),("modern".into(),"Latin Modern Math".into())],|p|&p.math_font,|p,v|p.math_font=v).description("内置 OpenType MATH 字体，独立于正文；支持分式、矩阵、mhchem 与 chemfig。"),
                self.number("LaTeX 字号",12.,36.,1.,|p|p.math_size,|p,v|p.math_size=v).description("默认 18px，独立于正文与代码块。"),
                SettingItem::render(|_,_,cx| div().text_sm().text_color(cx.theme().muted_foreground).child("行内：$E=mc^2$\n独立公式：独占行的 $$…$$\n化学式：\\ce{H2O} · 结构式：\\chemfig{…}")),
            ])),
        ];
        v_flex()
            .size_full()
            .refine_style(&ui_style)
            .text_size(px(ui_size as f32))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child("碧果设置"))
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new("pecan-settings")
                        .sidebar_width(px(190.))
                        .pages(pages),
                ),
            )
            .child(
                h_flex()
                    .h(px(30.))
                    .px_4()
                    .items_center()
                    .text_size(px(crate::ui::STATUS_SIZE))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        self.prefs
                            .read(cx)
                            .error
                            .clone()
                            .unwrap_or_else(|| "设置自动保存，立即生效".into()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui_kit::test]
    fn reading_navigation_toggles_and_keeps_the_active_page(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(SettingsPanel::new);
        visual.simulate_resize(size(px(900.), px(680.)));
        let draw = |visual: &mut gpui_kit::VisualTestContext| {
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        };
        draw(visual);
        for expanded in [true, false, true, false] {
            let row = visual.debug_bounds("sidebar-item-阅读").unwrap();
            visual.simulate_click(
                gpui_kit::point(row.left() + px(50.), row.top() + px(12.)),
                Default::default(),
            );
            draw(visual);
            assert_eq!(
                visual.debug_bounds("sidebar-item-正文排版").is_some(),
                expanded
            );
            assert_eq!(
                visual.debug_bounds("sidebar-item-代码块").is_some(),
                expanded
            );
            assert!(visual.debug_bounds("settings-content-阅读").is_some());
        }
    }

    #[test]
    fn installed_font_options_keep_all_public_families_and_remove_duplicates() {
        let options = font_options(vec![
            "DIN Alternate".into(),
            "Custom CJK".into(),
            "DIN Alternate".into(),
            ".Hidden".into(),
            "".into(),
        ]);
        let values: Vec<_> = options.iter().map(|(value, _)| value.as_ref()).collect();
        assert_eq!(
            values,
            [
                ".SystemUIFont",
                ".SystemMonospace",
                "Custom CJK",
                "DIN Alternate"
            ]
        );
    }
}
