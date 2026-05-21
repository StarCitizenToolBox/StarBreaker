use std::collections::HashMap;

use starbreaker_ui::{
    CanvasFetcher, PipelineInputs, StyleFetcher, SwfFetcher, UiBindingView,
    UiError, UiRendererHint, compile_ir_for_binding,
};
use starbreaker_ui::pipeline::AssetFetcher;

struct MockCanvasFetcher {
    by_guid: HashMap<String, serde_json::Value>,
}

impl CanvasFetcher for MockCanvasFetcher {
    fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, UiError> {
        self.by_guid
            .get(guid)
            .cloned()
            .ok_or_else(|| UiError::RenderError(format!("missing canvas guid: {guid}")))
    }

    fn fetch_canvas_by_name(&self, record_name: &str) -> Result<serde_json::Value, UiError> {
        self.by_guid
            .values()
            .find(|v| {
                v.get("_RecordName_")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n == record_name)
            })
            .cloned()
            .ok_or_else(|| UiError::RenderError(format!("missing canvas name: {record_name}")))
    }
}

struct DummySwfFetcher;

impl SwfFetcher for DummySwfFetcher {
    fn fetch_swf_bytes(&self, _p4k_path: &str) -> Result<Vec<u8>, UiError> {
        Err(UiError::RenderError("swf not required for IR compile test".to_string()))
    }
}

struct DummyStyleFetcher;

impl StyleFetcher for DummyStyleFetcher {
    fn fetch_manufacturer_style(
        &self,
        _manufacturer_id: &str,
    ) -> Result<starbreaker_ui::ManufacturerStyle, UiError> {
        Err(UiError::RenderError("style not required for IR compile test".to_string()))
    }
}

struct DummyAssetFetcher;

impl AssetFetcher for DummyAssetFetcher {
    fn fetch_image_bytes(&self, _p4k_path: &str) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn compile_ir_for_binding_uses_content_canvas_guid() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "size": {"x": 100.0, "y": 100.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "status_text",
                    "text": "READY",
                    "isActive": true,
                    "position": {"x": 10.0, "y": 20.0},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 60.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("Screen_Left_Upper_RTT"),
        default_view_index: None,
        default_screen_slot: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &canvas_fetcher,
        swf_fetcher: &swf_fetcher,
        style_fetcher: &style_fetcher,
        asset_fetcher: &asset_fetcher,
        target_size: (200, 100),
        apply_postprocess: false,
        localization_map: None,
        loc_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs).expect("IR should compile");
    assert_eq!(ir.canvas_guid, "content-guid");
    assert_eq!(ir.target_width, 200);
    assert_eq!(ir.target_height, 100);
    assert_eq!(ir.nodes.len(), 1);
    assert_eq!(ir.nodes[0].name, "status_text");
    assert_eq!(ir.renderer_hint, UiRendererHint::Bb);
}

#[test]
fn compile_ir_for_binding_matches_golden_fixture() {
    let root = serde_json::json!({
        "_RecordName_": "BuildingBlocks_Canvas.TestRoot",
        "_RecordValue_": {
            "size": {"x": 100.0, "y": 100.0},
            "scene": [
                {
                    "_Pointer_": "ptr:1",
                    "_Type_": "BuildingBlocks_WidgetTextField",
                    "name": "status_text",
                    "text": "READY",
                    "isActive": true,
                    "position": {"x": 10.0, "y": 20.0},
                    "sizing": {
                        "width": {"behavior": "Fixed", "value": 60.0},
                        "height": {"behavior": "Fixed", "value": 20.0}
                    }
                }
            ],
            "operations": []
        }
    });

    let mut by_guid = HashMap::new();
    by_guid.insert("content-guid".to_string(), root);

    let canvas_fetcher = MockCanvasFetcher { by_guid };
    let swf_fetcher = DummySwfFetcher;
    let style_fetcher = DummyStyleFetcher;
    let asset_fetcher = DummyAssetFetcher;

    let binding = UiBindingView {
        canvas_guid: Some("container-guid"),
        content_canvas_guid: Some("content-guid"),
        binding_kind: Some("mfd"),
        manufacturer_id: Some("drak"),
        helper_name: Some("Screen_Left_Upper_RTT"),
        default_view_index: None,
        default_screen_slot: None,
    };

    let inputs = PipelineInputs {
        binding: &binding,
        canvas_fetcher: &canvas_fetcher,
        swf_fetcher: &swf_fetcher,
        style_fetcher: &style_fetcher,
        asset_fetcher: &asset_fetcher,
        target_size: (200, 100),
        apply_postprocess: false,
        localization_map: None,
        loc_fetcher: None,
    };

    let ir = compile_ir_for_binding(&inputs).expect("IR should compile");
    let actual = serde_json::to_value(ir).expect("serialize actual");
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/ui_ir/expected_testroot_ir.json"
    ))
    .expect("parse expected fixture");

    assert_eq!(actual, expected);
}
