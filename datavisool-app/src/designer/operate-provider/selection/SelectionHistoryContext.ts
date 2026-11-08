/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
let selectionProvider: () => string[] = () => [];

export const setHistorySelectionProvider = (provider: () => string[]) => {
    selectionProvider = provider;
};

export const getHistorySelectionIds = () => [...selectionProvider()];
