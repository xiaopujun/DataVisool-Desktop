/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

abstract class AbstractConvert<P = any> {
    /**
     * 获取转换器唯一标识
     */
    abstract getKey(): string;

    /**
     * 正向转换
     * @param data
     */
    abstract convert(data: P): void;

    /**
     * 反向转换
     * @param data
     */
    abstract convertBack(data: P): Promise<void>
}

export default AbstractConvert;