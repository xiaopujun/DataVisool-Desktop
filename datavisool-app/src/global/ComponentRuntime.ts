/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import React from 'react';
import * as ReactDOM from 'react-dom';
import * as ReactDOMClient from 'react-dom/client';
import * as SDK from '@datavisool/sdk';
import * as DataVisoolTypes from '@datavisool/sdk/types';
import * as DataVisoolUI from '@datavisool/ui';
import * as BasicUI from '@datavisool/ui/basic';
import * as SchemaUI from '@datavisool/ui/schema';
import * as CodeEditorUI from '@datavisool/ui/code-editor';
import * as FeedbackUI from '@datavisool/ui/feedback';
import AbstractDesignerController from '@/framework/core/AbstractDesignerController';
import BPExecutor from '@/designer/blueprint/core/BPExecutor';

/** Runtime surface for component bundles. */
export const componentSdk = {
    ...SDK,
    AbstractDesignerController,
    BPExecutor,
} satisfies typeof SDK;

// Public subentries also expose helpers and icons absent from the original root.
export const componentUI = {...DataVisoolUI, ...BasicUI, ...SchemaUI, ...CodeEditorUI, ...FeedbackUI};

export function exportComponentRuntime(): void {
    window.React = React;
    // The CLI maps react-dom/client and react-dom to the same global.
    window.ReactDOM = {...ReactDOM, ...ReactDOMClient};
    window.DataVisoolSDK = componentSdk;
    window.DataVisoolTypes = DataVisoolTypes;
    window.DataVisoolUI = componentUI;
}
