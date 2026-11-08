/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import AbstractCache from "./AbstractCache";
import {IImageData} from "@/comps/builtin/base-image/BaseImageComponent";

class ImageSourceCache extends AbstractCache<any> {
    public getAllImageCache(): IImageData[] {
        return Array.from(this.getCachePool().values()) || [];
    }
}

const imageSourceCache = new ImageSourceCache();
export default imageSourceCache;
