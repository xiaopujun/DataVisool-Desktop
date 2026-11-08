/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import AbstractCache from "./AbstractCache";

class LocalCoverCache extends AbstractCache<any> {

    public getAllCoverCache(): string[] {
        return Array.from(this.getCachePool().values()) || [];
    }

    public clearCoverCache(): void {
        this.getAllCoverCache().forEach((url: string) => {
            URL.revokeObjectURL(url);
        });
    }
}

const localCoverCache = new LocalCoverCache();
export default localCoverCache;

