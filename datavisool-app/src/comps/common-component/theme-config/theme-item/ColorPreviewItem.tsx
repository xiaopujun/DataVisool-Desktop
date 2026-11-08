/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import React from 'react';
import './ColorPreviewItem.less';

interface ColorPreviewItemProps {
    color: string;
}

const ColorPreviewItem: React.FC<ColorPreviewItemProps> = ({ color }) => {
    return (
        <div className="datavisool-app-color-preview-item"
            style={{ background: color }}
            title={color} />
    );
};

export default ColorPreviewItem;
