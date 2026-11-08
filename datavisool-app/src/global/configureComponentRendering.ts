/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {setRenderErrorFallback} from '@datavisool/sdk/react';
import {RenderFaultPanel} from '@datavisool/ui/feedback';

// The SDK catches errors; the application supplies its presentation.
setRenderErrorFallback(RenderFaultPanel);
