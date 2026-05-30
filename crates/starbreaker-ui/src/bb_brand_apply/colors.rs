use crate::bb_scene::{BbBorder, BbNode, BbNodeType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorStyleRole {
    Foreground,
    Surface,
}

pub(super) fn color_style_role_for_field(field_name: &str, node: &BbNode) -> ColorStyleRole {
    if field_name.eq_ignore_ascii_case("FillColor")
        && matches!(node.ty, BbNodeType::WidgetCustomShape | BbNodeType::WidgetIcon | BbNodeType::WidgetImage)
        && color_overlay_enabled(node)
    {
        return ColorStyleRole::Foreground;
    }
    ColorStyleRole::Surface
}

fn color_overlay_enabled(node: &BbNode) -> bool {
    node.raw
        .get("enableColorOverlay")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || node
            .raw
            .get("svgFill")
            .and_then(|v| v.get("enableColorOverlay"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

pub(super) fn parse_color_value(
    value: &serde_json::Value,
    palette_source: &serde_json::Value,
    role: ColorStyleRole,
) -> Option<[f32; 4]> {
    if value
        .get("_Type_")
        .and_then(|ty| ty.as_str())
        .is_some_and(|ty| ty == "BuildingBlocks_ColorSolid")
    {
        return value
            .get("color")
            .and_then(|color| parse_color_value(color, palette_source, role));
    }

    if value.get("color").and_then(|v| v.as_str()).is_some() && value.get("r").is_none() {
        return parse_named_color(value, palette_source, role);
    }
    value.as_object().map(parse_literal_color)
}

fn parse_literal_color(color_obj: &serde_json::Map<String, serde_json::Value>) -> [f32; 4] {
    let r = color_obj
        .get("r")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let g = color_obj
        .get("g")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let b = color_obj
        .get("b")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let a = color_obj
        .get("a")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;

    if r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0 {
        [r / 255.0, g / 255.0, b / 255.0, a / 255.0]
    } else {
        [r, g, b, a]
    }
}

fn parse_named_color(
    value: &serde_json::Value,
    palette_source: &serde_json::Value,
    role: ColorStyleRole,
) -> Option<[f32; 4]> {
    let name = color_style_token(value)?;
    let alpha = value.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    let slot = color_style_slot_index(name, role)?;
    let color_styles = palette_source
        .get("colorStyles")
        .or_else(|| palette_source.get("_RecordValue_").and_then(|v| v.get("colorStyles")))?
        .as_array()?;
    let color_obj = color_styles.get(slot)?.get("color")?.as_object()?;
    let mut color = parse_literal_color(color_obj);
    color[3] *= alpha.clamp(0.0, 1.0);
    Some(color)
}

fn color_style_slot_index(name: &str, role: ColorStyleRole) -> Option<usize> {
    match name {
        "Bright" | "Base" => Some(0),
        "Accent1" if role == ColorStyleRole::Foreground => Some(0),
        "Accent2" | "Positive" | "Success" => Some(1),
        "Accent3" | "Warning" => Some(2),
        "Accent4" | "Critical" | "Negative" => Some(3),
        "Accent1" | "Accent5" | "ContactUnknown" => Some(4),
        "Mid" | "ContactNeutral" => Some(5),
        "Light" | "Disabled" => Some(6),
        "Highlight" | "ContactPositiveRep" => Some(7),
        "Surface" => Some(8),
        "BG" | "Background" => Some(9),
        "FG" | "Foreground" => Some(10),
        "Backlight" | "ContactAgressive" | "ContactAggressive" => Some(11),
        "PositiveState" => Some(12),
        "WarningState" => Some(13),
        "CriticalState" => Some(14),
        "Text" => Some(15),
        "Gold" | "Special" => Some(16),
        _ => None,
    }
}

pub(super) fn color_style_token(value: &serde_json::Value) -> Option<&str> {
    value
        .get("color")
        .and_then(|v| v.as_str())
        .filter(|name| !name.trim().is_empty())
}

/// Ensure `node.border` is `Some(…)`, initializing to default if `None`.
pub(super) fn ensure_border(node: &mut BbNode) {
    if node.border.is_none() {
        node.border = Some(BbBorder::default());
    }
}

/// Write a color to `node.raw` as an object `{r, g, b, a}`.
pub(super) fn write_color_to_raw(field_name: &str, color: [f32; 4], node: &mut BbNode) {
    let color_obj = serde_json::json!({
        "r": color[0],
        "g": color[1],
        "b": color[2],
        "a": color[3],
    });
    node.raw
        .as_object_mut()
        .and_then(|obj| obj.insert(field_name.to_string(), color_obj));
}

pub(super) fn write_color_token_to_raw(field_name: &str, token: Option<&str>, node: &mut BbNode) {
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    node.raw.as_object_mut().and_then(|obj| {
        obj.insert(
            format!("{field_name}Token"),
            serde_json::Value::String(token.to_string()),
        )
    });
}

