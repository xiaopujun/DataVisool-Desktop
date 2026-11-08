/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
//! Statically reviewed legacy interface mappings.
const INTERFACE: &[&str] = &[
    "AbstractController",
    "AbstractDefinition",
    "AbstractDesignerController",
    "AbstractDesignerDefinition",
    "BaseInfo",
    "AnimationConfig",
    "ThemeConfig",
    "DataConfig",
    "FilterConfig",
    "BPExecutor",
];
const UTIL: &[&str] = &["ComponentUtil", "ObjectUtil", "Base64Util", "ColorUtil", "FileUtil", "IdGenerate", "URLUtil"];
const LIGHT_UI: &[&str] = &["LCGUI"];
const UI: &[&str] = &[
    "Accordion",
    "Avatar",
    "Button",
    "Card",
    "CardPanel",
    "CheckBox",
    "CodeEditor",
    "normalizeEditorValue",
    "ColorMode",
    "ColorPicker",
    "ColorsPicker",
    "ControlGroup",
    "FrameLayout",
    "Dialog",
    "Dropdown",
    "Empty",
    "FullEditor",
    "GlobalMessage",
    "GlobalModal",
    "Grid",
    "GroupButton",
    "ImageUpload",
    "Input",
    "LCGUI",
    "LCGUIUtil",
    "LCGUIUtilDefault",
    "Loading",
    "Menu",
    "MonacoEditor",
    "NumberInput",
    "Pagination",
    "Progress",
    "Radio",
    "RangeSlider",
    "SchemaPathNode",
    "SearchableSelect",
    "Select",
    "Slider",
    "Space",
    "Spin",
    "SubAccordion",
    "Switch",
    "Tabs",
    "Tag",
    "TextArea",
    "TextOnly",
    "UIContainer",
    "UIMap",
    "Upload",
    "globalMessage",
    "globalModal",
    "buttonVariants",
    "cn",
    "LCPUIContext",
    "LCPUIProvider",
    "useLCPUI",
];
const SDK: &[&str] = &[
    "AbstractController",
    "AbstractDefinition",
    "AbstractDesignerController",
    "AbstractDesignerDefinition",
    "AnimationConfig",
    "BPExecutor",
    "Base64Util",
    "BaseInfo",
    "ColorUtil",
    "ComponentUtil",
    "DataConfig",
    "FileUtil",
    "FilterConfig",
    "IdGenerate",
    "ObjectUtil",
    "ThemeConfig",
    "URLUtil",
    "registerHostConfigPanels",
    "setRenderErrorFallback",
];
const TYPES: &[&str] = &["AnchorPointType", "DesignerMode", "DesignerScene", "ProjectState"];

pub(super) fn target(global: &str) -> Option<&'static str> {
    match global {
        "LightChaserInterface" | "LightChaserUtil" | "LCPSDK" => Some("DataVisoolSDK"),
        "LightChaserUI" | "LCPUI" => Some("DataVisoolUI"),
        "LCPTypes" => Some("DataVisoolTypes"),
        _ => None,
    }
}

pub(super) fn allowed(global: &str, member: &str) -> bool {
    let members = match global {
        "LightChaserInterface" => INTERFACE,
        "LightChaserUtil" => UTIL,
        "LightChaserUI" => LIGHT_UI,
        "LCPUI" => UI,
        "LCPSDK" => SDK,
        "LCPTypes" => TYPES,
        _ => return false,
    };
    members.contains(&member)
}

pub(super) fn ui_name(member: &str) -> Option<&'static str> {
    match member {
        "LCGUI" => Some("DatavisoolGUI"),
        "LCGUIUtil" => Some("DatavisoolGUIUtil"),
        "LCGUIUtilDefault" => Some("DatavisoolGUIUtilDefault"),
        "LCPUIContext" => Some("DataVisoolUIContext"),
        "LCPUIProvider" => Some("DataVisoolUIProvider"),
        "useLCPUI" => Some("useDataVisoolUI"),
        _ => None,
    }
}

pub(super) fn util_method(member: &str, method: &str) -> bool {
    match member {
        "ComponentUtil" => ["createAndRender", "unmount"].contains(&method),
        "ObjectUtil" => ["merge", "getOriginValue", "removeEmptyKeys", "objToCodeStr"].contains(&method),
        "Base64Util" => ["toBase64", "fromBase64"].contains(&method),
        "ColorUtil" => ["colorConversion", "hexToRgba", "rgbaToHex", "parseLinerGradient", "parseRadialGradient"].contains(&method),
        "FileUtil" => ["blobToBase64", "base64ToBlob"].contains(&method),
        "IdGenerate" => method == "generateId",
        "URLUtil" => method == "parseUrlParams",
        _ => false,
    }
}

// TODO(legacy-compat): Remove when imported legacy components are retired.
pub(super) const LEGACY_MERGE: &str = r#"(function datavisoolLegacyMerge(originalData,newData){if(!originalData&&!newData)return null;if(!originalData&&newData)return newData;if(originalData&&!newData)return originalData;Object.keys(newData).forEach(function(key){var value=newData[key];if(Object.prototype.hasOwnProperty.call(originalData,key)){if(Array.isArray(value))originalData[key]=value;else if(value&&typeof value==="object"&&Object.keys(value).length>0&&originalData[key]&&typeof originalData[key]==="object")datavisoolLegacyMerge(originalData[key],value);else originalData[key]=value;}else originalData[key]=value;});return originalData;})"#;
