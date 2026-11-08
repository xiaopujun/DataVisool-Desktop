/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {useNavigate} from "react-router-dom";
import {Button} from "@datavisool/ui";

export default function NotFound() {
    const navigate = useNavigate();
    return (
        <div style={{minHeight: '100vh', display: 'grid', placeItems: 'center', padding: 24, textAlign: 'center'}}>
            <div>
                <div style={{fontSize: 56, fontWeight: 700, color: '#4FB8FF', marginBottom: 12}}>404</div>
                <div style={{fontSize: 16, color: '#A0A0B2', marginBottom: 24}}>找不到页面</div>
                <Button type="primary" onClick={() => navigate('/home')}>返回首页</Button>
            </div>
        </div>
    );
}
