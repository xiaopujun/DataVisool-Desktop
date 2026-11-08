/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
import {expect, it} from 'vitest';
import {componentUI, componentSdk} from '@/global/ComponentRuntime';
import * as types from '@datavisool/sdk/types';

it('exposes the reviewed legacy conversion targets in the actual Desktop component runtime', () => {
    for (const member of ["Accordion", "Avatar", "Button", "Card", "CardPanel", "CheckBox", "CodeEditor", "normalizeEditorValue", "ColorMode", "ColorPicker", "ColorsPicker", "ControlGroup", "FrameLayout", "Dialog", "Dropdown", "Empty", "FullEditor", "GlobalMessage", "GlobalModal", "Grid", "GroupButton", "ImageUpload", "Input", "DatavisoolGUI", "DatavisoolGUIUtil", "DatavisoolGUIUtilDefault", "Loading", "Menu", "MonacoEditor", "NumberInput", "Pagination", "Progress", "Radio", "RangeSlider", "SchemaPathNode", "SearchableSelect", "Select", "Slider", "Space", "Spin", "SubAccordion", "Switch", "Tabs", "Tag", "TextArea", "TextOnly", "UIContainer", "UIMap", "Upload", "globalMessage", "globalModal", "buttonVariants", "cn", "DataVisoolUIContext", "DataVisoolUIProvider", "useDataVisoolUI"] as const) expect((componentUI as any)[member], member).toBeDefined();
    for (const member of ["AbstractController", "AbstractDefinition", "AbstractDesignerController", "AbstractDesignerDefinition", "AnimationConfig", "BPExecutor", "Base64Util", "BaseInfo", "ColorUtil", "ComponentUtil", "DataConfig", "FileUtil", "FilterConfig", "IdGenerate", "ObjectUtil", "ThemeConfig", "URLUtil", "registerHostConfigPanels", "setRenderErrorFallback"] as const) expect((componentSdk as any)[member], member).toBeDefined();
    for (const member of ["AnchorPointType", "DesignerMode", "DesignerScene", "ProjectState"] as const) expect((types as any)[member], member).toBeDefined();
});
