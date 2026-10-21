use super::*;
use crate::error::UiError;
use serde_json::json;

fn guid_a() -> &'static str {
    "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
}
fn guid_b() -> &'static str {
    "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
}
fn guid_c() -> &'static str {
    "cccccccc-cccc-cccc-cccc-cccccccccccc"
}

#[test]
fn parse_canvas_with_text_field_operation() {
    let json = json!({
        "views": [],
        "scene": [
            {
                "_Type_": "BuildingBlocks_TextField",
                "binding": "/vehicle/targetname",
                "text": "NO TARGET",
                "fontId": "font_amber_mono"
            }
        ],
        "operations": [
            {
                "_Type_": "BuildingBlocks_BindingsStringVariable",
                "binding": "/vehicle/targetname",
                "property": "text",
                "defaultValue": "NO TARGET"
            }
        ]
    });

    let record = CanvasParser::parse(guid_a(), "TestCanvas", &json).unwrap();

    assert_eq!(record.guid, guid_a());
    assert_eq!(record.name, "TestCanvas");
    assert_eq!(record.scene.len(), 1);
    assert_eq!(record.scene[0].kind, "BuildingBlocks_TextField");

    assert_eq!(record.operations.len(), 1);
    let op = &record.operations[0];
    assert_eq!(op.kind, "BuildingBlocks_BindingsStringVariable");
    assert_eq!(op.binding_path.as_deref(), Some("/vehicle/targetname"));
    assert_eq!(op.target_property.as_deref(), Some("text"));
    assert!(matches!(op.default_value, Some(Value::Str(ref s)) if s == "NO TARGET"));
}

#[test]
fn classify_text_field_component() {
    let item = json!({
        "_Type_": "BuildingBlocks_TextField",
        "binding": "/vehicle/targetname",
        "text": ">> NO TARGET <<",
        "fontId": "font_amber",
        "color": 4294956800u64
    });

    let component = CanvasParser::parse_view_component(&item);
    match component {
        ViewComponent::TextField {
            binding_path,
            default_text,
            font_id,
            color,
        } => {
            assert_eq!(binding_path.as_deref(), Some("/vehicle/targetname"));
            assert_eq!(default_text.as_deref(), Some(">> NO TARGET <<"));
            assert_eq!(font_id.as_deref(), Some("font_amber"));
            assert!(color.is_some());
        }
        other => panic!("Expected TextField, got {:?}", other),
    }
}

#[test]
fn parse_canvas_with_widget_canvas_scene_item() {
    let json = json!({
        "views": [],
        "scene": [
            {
                "_Type_": "BuildingBlocks_WidgetCanvas",
                "canvas": guid_b(),
                "urlPostfix": "mfd"
            }
        ],
        "operations": []
    });

    let record = CanvasParser::parse(guid_a(), "ContainerCanvas", &json).unwrap();
    assert_eq!(record.scene.len(), 1);
    let item = &record.scene[0];
    assert_eq!(item.kind, "BuildingBlocks_WidgetCanvas");
    assert_eq!(item.guid.as_deref(), Some(guid_b()));
    assert_eq!(item.url_postfix.as_deref(), Some("mfd"));
}

#[test]
fn resolver_descends_into_sub_canvas() {
    let root_json = json!({
        "scene": [
            {
                "_Type_": "BuildingBlocks_WidgetCanvas",
                "canvas": guid_b()
            }
        ],
        "operations": []
    });

    let child_json = json!({
        "__name": "ChildCanvas",
        "scene": [
            {
                "_Type_": "BuildingBlocks_TextField",
                "text": "child text"
            }
        ],
        "operations": []
    });

    let resolver = CanvasWidgetTreeResolver::new();
    let result = resolver
        .resolve(guid_a(), |guid| -> Result<_, std::convert::Infallible> {
            if guid == guid_a() {
                Ok(root_json.clone())
            } else if guid == guid_b() {
                Ok(child_json.clone())
            } else {
                panic!("unexpected GUID {}", guid)
            }
        })
        .unwrap();

    assert_eq!(result.root.guid, guid_a());
    assert!(result.children.contains_key(guid_b()), "child canvas missing");
    let child = &result.children[guid_b()];
    assert_eq!(child.name, "ChildCanvas");
    assert_eq!(child.scene.len(), 1);
}

#[test]
fn resolver_detects_cycle() {
    let a_json = json!({
        "scene": [{ "_Type_": "BuildingBlocks_WidgetCanvas", "canvas": guid_b() }],
        "operations": []
    });
    let b_json = json!({
        "scene": [{ "_Type_": "BuildingBlocks_WidgetCanvas", "canvas": guid_a() }],
        "operations": []
    });

    let resolver = CanvasWidgetTreeResolver::new();
    let result = resolver.resolve(guid_a(), |guid| -> Result<_, std::convert::Infallible> {
        if guid == guid_a() {
            Ok(a_json.clone())
        } else {
            Ok(b_json.clone())
        }
    });

    assert!(
        matches!(result, Err(UiError::CycleDetected(_))),
        "expected CycleDetected, got {:?}",
        result
    );
}

#[test]
fn parse_views_with_screens() {
    let json = json!({
        "views": [
            {
                "name": "_mfd",
                "screens": [ guid_b(), guid_c() ]
            },
            {
                "name": "Off",
                "screens": []
            }
        ],
        "scene": [],
        "operations": []
    });

    let record = CanvasParser::parse(guid_a(), "MFDCanvas", &json).unwrap();
    assert_eq!(record.views.len(), 2);

    let first_view = &record.views[0];
    assert_eq!(first_view.name, "_mfd");
    assert_eq!(first_view.ordinal, 0);
    assert!(first_view.default);
    assert_eq!(first_view.components.len(), 2);
    for comp in &first_view.components {
        assert!(matches!(comp, ViewComponent::WidgetCanvas { .. }));
    }

    let second_view = &record.views[1];
    assert_eq!(second_view.name, "Off");
    assert!(!second_view.default);
}

#[test]
fn parse_empty_canvas() {
    let json = json!({ "views": [], "scene": [], "operations": [] });
    let record = CanvasParser::parse("00000000-0000-0000-0000-000000000000", "Empty", &json)
        .unwrap();
    assert!(record.views.is_empty());
    assert!(record.scene.is_empty());
    assert!(record.operations.is_empty());
}

#[test]
fn parse_non_object_is_error() {
    let json = json!([1, 2, 3]);
    let result = CanvasParser::parse("guid", "name", &json);
    assert!(matches!(result, Err(UiError::ParseError(_))));
}

#[test]
fn parse_scene_item_transform() {
    let json = json!({
        "views": [],
        "scene": [
            {
                "_Type_": "BuildingBlocks_Shape",
                "transform": { "tx": 100.0, "ty": 50.0, "sx": 2.0, "sy": 0.5, "angle": 45.0 }
            }
        ],
        "operations": []
    });

    let record = CanvasParser::parse(guid_a(), "ShapeCanvas", &json).unwrap();
    let item = &record.scene[0];
    assert!((item.transform.tx - 100.0).abs() < 1e-5);
    assert!((item.transform.ty - 50.0).abs() < 1e-5);
    assert!((item.transform.sx - 2.0).abs() < 1e-5);
    assert!((item.transform.angle - 45.0).abs() < 1e-5);
}

#[test]
fn resolver_max_depth_exceeded() {
    let resolver = CanvasWidgetTreeResolver::with_max_depth(1);

    let result = resolver.resolve(guid_a(), |guid| -> Result<_, std::convert::Infallible> {
        let next = match guid {
            g if g == guid_a() => guid_b(),
            g if g == guid_b() => guid_c(),
            _ => return Ok(json!({ "scene": [], "operations": [] })),
        };
        Ok(json!({
            "scene": [{ "_Type_": "BuildingBlocks_WidgetCanvas", "canvas": next }],
            "operations": []
        }))
    });

    assert!(
        matches!(result, Err(UiError::MaxDepthExceeded { .. })),
        "expected MaxDepthExceeded, got {:?}",
        result
    );
}
