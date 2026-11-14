//! The AI Drawing docker: the connection to the AI service (service,
//! address, model, key), the request in words, Draw, and what came back.
//! The work is in `ai.rs`; see docs/behavior/ai-drawing.md.

use crate::ai::{AiStatus, Provider};
use crate::app::{App, DockerTab};
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::ui::tips::RichTip;
use egui::{RichText, Ui};

const ERROR: egui::Color32 = egui::Color32::from_rgb(0xC4, 0x2B, 0x1C);
/// The connection fields are at least this wide; a panel too narrow for
/// a label and such a field beside it puts each label above its field.
const MIN_FIELD_W: f32 = 110.0;
/// The connection fields are at most this wide (room for a long address
/// in Tools > Options > AI, without stretching across a wide dialog).
const MAX_FIELD_W: f32 = 360.0;
/// The gap between a label and its field, and between two rows.
const ROW_GAP: f32 = 8.0;
/// Width of the button that opens a model list, with its gap.
const LIST_BUTTON_W: f32 = 26.0;

/// How the rows of the connection are laid out in the room there is
/// (#2040): the labels in a column as wide as the longest of them and
/// the fields in what is left, or, in a narrow panel, each label above
/// its field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rows {
    /// Width of the label column (not used when stacked).
    pub label_w: f32,
    /// Width of each field.
    pub field_w: f32,
    /// Each label sits above its field.
    pub stacked: bool,
}

impl Rows {
    /// The layout for `available` points of width and labels of which
    /// the longest is `label_w` wide.
    pub fn fit(available: f32, label_w: f32) -> Rows {
        let beside = available - label_w - ROW_GAP;
        let stacked = beside < MIN_FIELD_W;
        let room = if stacked { available } else { beside };
        Rows {
            label_w,
            field_w: room.clamp(MIN_FIELD_W.min(available.max(1.0)), MAX_FIELD_W),
            stacked,
        }
    }

    /// The layout for the room `ui` has and these labels.
    fn of(ui: &Ui, labels: &[String]) -> Rows {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let label_w = labels
            .iter()
            .map(|l| {
                ui.painter()
                    .layout_no_wrap(l.clone(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
            .ceil();
        Rows::fit(ui.available_width(), label_w)
    }

    /// One row: `label`, then what `field` adds, which is told how wide
    /// to be.
    fn row<R>(&self, ui: &mut Ui, label: &str, field: impl FnOnce(&mut Ui, f32) -> R) -> R {
        let out = if self.stacked {
            ui.label(label);
            field(ui, self.field_w)
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = ROW_GAP;
                let h = ui.spacing().interact_size.y;
                ui.allocate_ui_with_layout(
                    egui::vec2(self.label_w, h),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| ui.label(label),
                );
                field(ui, self.field_w)
            })
            .inner
        };
        ui.add_space((ROW_GAP - ui.spacing().item_spacing.y).max(0.0));
        out
    }
}

fn dim(ui: &mut Ui, text: String) {
    ui.label(RichText::new(text).color(Tokens::text_dim()).size(11.0));
}

pub fn ai_drawing(app: &mut App, ui: &mut Ui) {
    if !app.ai_features_on() {
        crate::ui::ai_ui::banner(app, ui, crate::ui::ai_ui::Need::Drawing);
    }
    // The connection, open until a key is set where one is needed.
    let provider = app.settings.ai.provider;
    let needs_setup = provider.needs_key() && app.ai.key.trim().is_empty();
    egui::CollapsingHeader::new(tr("ai.connection"))
        .id_salt("ai_connection")
        .default_open(needs_setup)
        .show(ui, |ui| connection(app, ui));
    ui.add_space(4.0);
    ui.label(tr("ai.request"));
    let field = ui.add(
        egui::TextEdit::multiline(&mut app.ai.request)
            .id(egui::Id::new("ai_request"))
            .hint_text(tr("ai.request_hint"))
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    // Ctrl+Enter in the request draws.
    let ctrl_enter =
        field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
    // Wrapped: in a narrow panel New conversation goes under Draw.
    ui.horizontal_wrapped(|ui| {
        if app.ai.busy() {
            ui.spinner();
            if ui
                .button(tr("dialog.cancel"))
                .rich_tip(crate::ui::tips::keyed("dialog.cancel"))
                .clicked()
            {
                app.ai_cancel();
            }
        } else if ui
            .add_enabled(app.ai_features_on(), egui::Button::new(tr("ai.draw")))
            .clicked()
            || (ctrl_enter && app.ai_features_on())
        {
            app.ai_draw();
        }
        if ui
            .add_enabled(
                !app.ai.history.is_empty(),
                egui::Button::new(tr("ai.new_conversation")),
            )
            .clicked()
        {
            app.ai_new_conversation();
        }
    });
    match &app.ai.status {
        AiStatus::Idle => {}
        AiStatus::Waiting => dim(ui, app.ai.waiting_text()),
        AiStatus::Retrying => dim(ui, tr("ai.retrying")),
        AiStatus::Done(n) => {
            ui.label(trf("ai.done", &[("n", &n.to_string())]));
        }
        AiStatus::Listed(n) => dim(ui, trf("ai.models_found", &[("n", &n.to_string())])),
        AiStatus::Failed(e) => {
            ui.label(RichText::new(e.as_str()).color(ERROR));
        }
    }
    for line in app.ai.output.clone() {
        dim(ui, line);
    }
    if !app.ai.script.is_empty() {
        egui::CollapsingHeader::new(tr("ai.script"))
            .id_salt("ai_script")
            .show(ui, |ui| {
                let mut shown = app.ai.script.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut shown)
                        .code_editor()
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
                if ui.button(tr("ai.open_in_scripts")).clicked() {
                    app.script_source = app.ai.script.clone();
                    app.open_docker(DockerTab::Scripts);
                }
            });
    }
    ui.add_space(6.0);
    ui.separator();
    dim(ui, tr("ai.privacy"));
}

/// Service, address, model and key, and whether the key is remembered.
/// The rows follow the room there is ([`Rows`]): the fields stretch with
/// a wide panel and each label goes above its field in a narrow one.
pub fn connection(app: &mut App, ui: &mut Ui) {
    // The models to pick from: the service's own once listed, else the
    // usual ones of its kind.
    let choices = app.ai_model_choices();
    let hidden = app.ai_models_hidden();
    let mut fetch = false;
    let prefs = &mut app.settings.ai;
    let anthropic = prefs.provider == Provider::Anthropic;
    let pictures = prefs.can_make_pictures();
    let mut labels = vec![
        tr("aipreset.label"),
        tr("ai.provider"),
        tr("ai.endpoint"),
        tr("ai.model"),
        tr("ai.key"),
    ];
    if anthropic {
        labels.push(tr("ai.max_tokens"));
    }
    if pictures {
        labels.push(tr("ai.image_model"));
    }
    let rows = Rows::of(ui, &labels);

    rows.row(ui, &tr("aipreset.label"), |ui, w| {
        let current = crate::ai::preset_of(&*prefs);
        let shown = current
            .and_then(|i| crate::ai::PRESETS.get(i))
            .map(|(k, _, _)| tr(k))
            .unwrap_or_else(|| tr("aipreset.custom"));
        crate::ui::keynav::ComboBox::from_id_salt("ai_preset")
            .width(w)
            .truncate()
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (i, (key, provider, endpoint)) in crate::ai::PRESETS.iter().enumerate() {
                    if ui.selectable_label(current == Some(i), tr(key)).clicked() {
                        prefs.provider = *provider;
                        prefs.endpoint = endpoint.to_string();
                        prefs.model.clear();
                        prefs.image_model.clear();
                        app.ai.models.clear();
                        app.ai_image.models.clear();
                    }
                }
            })
            .response
            .rich_tip(tr("aipreset.tip"));
    });
    rows.row(ui, &tr("ai.provider"), |ui, w| {
        let mut provider = prefs.provider;
        let tip = crate::ui::tips::pair("ai.provider", "ai.provider_tip");
        if crate::ui::tips::tip_over(ui, tip, |ui| {
            let mut changed = false;
            crate::ui::keynav::ComboBox::from_id_salt("ai_provider")
                .width(w)
                .truncate()
                .selected_text(tr(provider.key()))
                .show_ui(ui, |ui| {
                    for p in Provider::ALL {
                        if ui.selectable_label(provider == p, tr(p.key())).clicked() {
                            provider = p;
                            changed = true;
                        }
                    }
                });
            changed
        }) {
            // Another kind of service: its own address and model.
            prefs.provider = provider;
            prefs.endpoint.clear();
            prefs.model.clear();
            prefs.image_model.clear();
            app.ai_image.models.clear();
        }
    });
    rows.row(ui, &tr("ai.endpoint"), |ui, w| {
        ui.add(
            egui::TextEdit::singleline(&mut prefs.endpoint)
                .hint_text(prefs.provider.default_endpoint())
                .desired_width(w),
        )
        .rich_tip(crate::ui::tips::pair("ai.endpoint", "ai.endpoint_tip"));
    });
    rows.row(ui, &tr("ai.model"), |ui, w| {
        let model_hint = match prefs.provider.default_model() {
            "" => tr("ai.model_hint"),
            m => m.to_string(),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add(
                egui::TextEdit::singleline(&mut prefs.model)
                    .hint_text(model_hint)
                    .desired_width(w - LIST_BUTTON_W),
            )
            .rich_tip(crate::ui::tips::pair("ai.model", "ai.model_tip"));
            ui.menu_button("\u{23F7}", |ui| {
                for m in &choices {
                    if ui.selectable_label(prefs.model == *m, m).clicked() {
                        prefs.model = m.clone();
                        ui.close();
                    }
                }
                if !choices.is_empty() {
                    ui.separator();
                }
                // The models that cannot draw stay out unless asked for.
                if hidden {
                    ui.checkbox(&mut app.ai.show_all_models, tr("ai.show_all_models"));
                }
                if ui.button(tr("ai.refresh_models")).clicked() {
                    fetch = true;
                    ui.close();
                }
            })
            .response
            .rich_tip(crate::ui::tips::pair("ai.model", "ai.model_list_tip"));
        });
    });
    rows.row(ui, &tr("ai.key"), |ui, w| {
        let key = ui
            .add(
                egui::TextEdit::singleline(&mut app.ai.key)
                    .password(true)
                    .desired_width(w),
            )
            .rich_tip(crate::ui::tips::pair("ai.key", "ai.key_tip"));
        if key.changed() && prefs.remember_key {
            prefs.key = app.ai.key.clone();
        }
    });
    // The longest reply, which the Messages API asks for (#1011).
    if anthropic {
        rows.row(ui, &tr("ai.max_tokens"), |ui, w| {
            let mut tokens = prefs.max_tokens();
            if ui
                .add_sized(
                    [w.min(MIN_FIELD_W), ui.spacing().interact_size.y],
                    crate::ui::field::NumField::new(&mut tokens)
                        .range(crate::ai::MIN_MAX_TOKENS..=crate::ai::MAX_MAX_TOKENS),
                )
                .rich_tip(crate::ui::tips::pair("ai.max_tokens", "ai.max_tokens_tip"))
                .changed()
            {
                prefs.max_tokens = tokens;
            }
        });
    }
    // Pictures: only for a service that can make them.
    if pictures {
        rows.row(ui, &tr("ai.image_model"), |ui, w| {
            image_model_field(prefs, &app.ai_image.models, ui, w);
        });
    } else {
        dim(ui, tr("aiui.drawings_only"));
    }
    ui.checkbox(&mut prefs.show_bar, tr("ai.show_bar"))
        .rich_tip(crate::ui::tips::pair("ai.show_bar", "ai.show_bar_tip"));
    if ui
        .checkbox(&mut prefs.remember_key, tr("ai.remember_key"))
        .rich_tip(crate::ui::tips::pair(
            "ai.remember_key",
            "ai.remember_key_tip",
        ))
        .changed()
    {
        prefs.key = if prefs.remember_key {
            app.ai.key.clone()
        } else {
            String::new()
        };
        app.settings.save();
    }
    dim(ui, tr("ai.local_hint"));
    if fetch {
        app.ai_fetch_models();
    }
}

/// The Image model field, `width` wide with its list button: typed, or
/// picked from the service's image models (once tested) or the usual
/// ones.
fn image_model_field(prefs: &mut crate::ai::AiPrefs, listed: &[String], ui: &mut Ui, width: f32) {
    use crate::ai::ImageSupport;
    use crate::ai_image::ImageProvider;
    let support = prefs.image_support();
    let (hint, usual): (String, &[&str]) = match support {
        ImageSupport::OpenAi => (
            ImageProvider::OpenAi.default_model().to_string(),
            ImageProvider::OpenAi.preset_models(),
        ),
        ImageSupport::Gemini => (
            ImageProvider::Gemini.default_model().to_string(),
            ImageProvider::Gemini.preset_models(),
        ),
        _ => (tr("ai.image_model_none"), &[]),
    };
    // The image model set up stays in the list once the service's own
    // list came (it may name it otherwise, or not at all).
    let choices = crate::ai::model_choices(&[prefs.image_model.clone()], listed, usual);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add(
            egui::TextEdit::singleline(&mut prefs.image_model)
                .hint_text(hint)
                .desired_width(width - LIST_BUTTON_W),
        )
        .rich_tip(tr("ai.image_model_tip"));
        ui.add_enabled_ui(!choices.is_empty(), |ui| {
            ui.menu_button("\u{23F7}", |ui| {
                for m in &choices {
                    if ui.selectable_label(prefs.image_model == *m, m).clicked() {
                        prefs.image_model = m.clone();
                        ui.close();
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{vec2, Pos2, Rect};

    fn frame(ctx: &egui::Context, app: &mut App) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| ai_drawing(app, ui));
        out.textures_delta.clear();
    }

    /// The connection's rows follow the room there is (#2040): in the
    /// Options dialog the labels sit beside fields that stretch up to
    /// their most, in a docker the fields take what the labels leave,
    /// and in a narrow one each label goes above a field as wide as the
    /// panel. A row is never wider than the room.
    #[test]
    fn the_connection_rows_follow_the_room() {
        let wide = Rows::fit(600.0, 90.0);
        assert!(!wide.stacked);
        assert_eq!(wide.field_w, MAX_FIELD_W);
        let docker = Rows::fit(320.0, 90.0);
        assert!(!docker.stacked);
        assert_eq!(docker.field_w, 320.0 - 90.0 - ROW_GAP);
        // The fixed 170 points of before no longer fit here: stacked.
        let narrow = Rows::fit(190.0, 90.0);
        assert!(narrow.stacked);
        assert_eq!(narrow.field_w, 190.0);
        let tiny = Rows::fit(60.0, 90.0);
        assert!(tiny.stacked);
        assert_eq!(tiny.field_w, 60.0);
        for available in [60.0, 150.0, 208.0, 209.0, 340.0, 900.0] {
            let r = Rows::fit(available, 90.0);
            let row = if r.stacked {
                r.field_w
            } else {
                r.label_w + ROW_GAP + r.field_w
            };
            assert!(row <= available, "{available}: {r:?}");
        }
    }

    #[test]
    fn the_docker_draws_in_every_state() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        frame(&ctx, &mut app);
        app.settings.ai.provider = Provider::OpenAiCompatible;
        app.ai.status = AiStatus::Waiting;
        frame(&ctx, &mut app);
        app.ai.status = AiStatus::Failed("no".into());
        app.ai.script = "print(1)".into();
        app.ai.output = vec!["1".into()];
        frame(&ctx, &mut app);
        app.ai.status = AiStatus::Done(3);
        frame(&ctx, &mut app);
        assert_eq!(app.ai.script, "print(1)", "the script is shown, not edited");
    }

    /// One frame of the whole window with `events`; the widgets as the
    /// accessibility tree has them: role, label and place.
    fn window(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> Vec<(egui::accesskit::Role, String, Rect)> {
        ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| {
                        let b = n.bounds()?;
                        Some((
                            n.role(),
                            n.label().unwrap_or_default().to_string(),
                            Rect::from_min_max(
                                Pos2::new(b.x0 as f32, b.y0 as f32),
                                Pos2::new(b.x1 as f32, b.y1 as f32),
                            ),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn click(ctx: &egui::Context, app: &mut App, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        window(ctx, app, vec![egui::Event::PointerMoved(at)]);
        window(ctx, app, vec![button(true)]);
        window(ctx, app, vec![button(false)]);
        window(ctx, app, Vec::new());
    }

    /// The Draw buttons of the window, top to bottom: the panel's, then
    /// the prompt bar's above the palette.
    fn draw_buttons(ctx: &egui::Context, app: &mut App) -> Vec<Rect> {
        let mut found: Vec<Rect> = window(ctx, app, Vec::new())
            .into_iter()
            .filter(|(role, label, _)| {
                *role == egui::accesskit::Role::Button && *label == tr("ai.draw")
            })
            .map(|(_, _, r)| r)
            .collect();
        found.sort_by(|a, b| a.top().total_cmp(&b.top()));
        found
    }

    /// Draw with words has two Draw buttons, the panel's and the prompt
    /// bar's (#2038, which could not be repeated): words typed in the
    /// panel's box and the panel's Draw pressed with the mouse send the
    /// request, with the prompt bar showing too; so do words typed in
    /// the bar and its Draw.
    #[test]
    fn the_panels_draw_button_sends_what_was_typed_in_the_panel() {
        use egui::accesskit::Role;
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.ai = Default::default();
        app.settings.ai_enabled = true;
        app.show_welcome = false;
        app.ai.key = "k".into();
        app.ai.fake = Some(|_| {
            Ok(crate::ai::HttpResponse {
                status: 200,
                retry_after: None,
                body: serde_json::json!({
                    "content": [{"type": "text", "text": "```javascript\nprint(1)\n```"}],
                    "stop_reason": "end_turn",
                })
                .to_string(),
            })
        });
        app.open_ai_drawing();
        for _ in 0..3 {
            window(&ctx, &mut app, Vec::new());
        }
        assert!(crate::ui::ai_bar::visible(&app), "the prompt bar shows too");
        let buttons = draw_buttons(&ctx, &mut app);
        assert_eq!(buttons.len(), 2, "the panel's Draw and the bar's");
        // Type into the panel's box, then press the panel's Draw.
        let field = window(&ctx, &mut app, Vec::new())
            .into_iter()
            .find(|(role, _, _)| *role == Role::MultilineTextInput)
            .map(|(_, _, r)| r)
            .expect("the panel's request box");
        click(&ctx, &mut app, field.center());
        window(
            &ctx,
            &mut app,
            vec![egui::Event::Text("a red circle".into())],
        );
        assert_eq!(app.ai.request, "a red circle");
        let buttons = draw_buttons(&ctx, &mut app);
        click(&ctx, &mut app, buttons[0].center());
        assert_eq!(app.ai.sent.len(), 1, "the panel's Draw sent the request");
        assert!(app.ai.sent[0].body.contains("a red circle"));
        assert_eq!(app.ai.status, AiStatus::Done(0));
        // The same from the bar: its field is the one on its Draw
        // button's row.
        let bar_draw = draw_buttons(&ctx, &mut app)[1];
        let bar = window(&ctx, &mut app, Vec::new())
            .into_iter()
            .find(|(role, _, r)| {
                *role == Role::TextInput && r.y_range().contains(bar_draw.center().y)
            })
            .map(|(_, _, r)| r)
            .expect("the bar's request field");
        click(&ctx, &mut app, bar.center());
        window(
            &ctx,
            &mut app,
            vec![egui::Event::Text("a blue square".into())],
        );
        assert_eq!(app.ai.request, "a blue square");
        let buttons = draw_buttons(&ctx, &mut app);
        click(&ctx, &mut app, buttons[1].center());
        assert_eq!(app.ai.sent.len(), 2, "the bar's Draw sent the request");
        assert!(app.ai.sent[1].body.contains("a blue square"));
    }

    #[test]
    fn the_key_is_kept_only_when_asked() {
        let mut app = App::headless();
        app.ai.key = "secret".into();
        assert!(app.settings.ai.key.is_empty());
        let json = serde_json::to_string(&app.settings).expect("settings");
        assert!(!json.contains("secret"));
        // Remembered: it goes with the preferences, and comes back.
        app.settings.ai.remember_key = true;
        app.settings.ai.key = app.ai.key.clone();
        let json = serde_json::to_string(&app.settings).expect("settings");
        assert!(json.contains("secret"));
        let back: crate::settings::Settings = serde_json::from_str(&json).expect("read");
        assert_eq!(crate::ai::remembered_key(&back.ai), "secret");
        // A key in a file that does not say to remember it is not used.
        let mut odd = back.ai.clone();
        odd.remember_key = false;
        assert_eq!(crate::ai::remembered_key(&odd), "");
    }
}
