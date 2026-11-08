/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {Button, Dialog, Grid, Input, NumberInput, Switch, globalMessage} from "@datavisool/ui";
import {observer} from "mobx-react";
import releaseStore from "@/pages/home/release-management/ReleaseStore.ts";
import {useState} from "react";
import {tx} from "@/i18n/miniI18n.ts";

const ReleaseEditPanel = observer(() => {
    const {currentItem} = releaseStore;
    const [name, setName] = useState(currentItem.name ?? "");
    const [expiration, setExpiration] = useState<number | undefined>(currentItem.expirationDate ?? undefined);
    const [encrypt, setEncrypt] = useState(Boolean(currentItem.encrypt));
    const [password, setPassword] = useState("");
    const [saving, setSaving] = useState(false);
    const [wasEncrypted] = useState(Boolean(currentItem.encrypt));

    const close = () => {
        if (!saving) releaseStore.setModalVisible(false);
    };

    const doUpdate = async () => {
        if (saving) return;
        if (!name.trim()) {
            globalMessage.messageApi?.error(tx("名称不能为空"));
            return;
        }
        if (expiration !== undefined && (!Number.isInteger(expiration) || expiration < 0)) {
            globalMessage.messageApi?.error(tx("有效期必须是非负整数"));
            return;
        }
        if (encrypt && !wasEncrypted && !password) {
            globalMessage.messageApi?.error(tx("请输入密码！"));
            return;
        }
        releaseStore.setCurrentItem({
            id: currentItem.id, name: name.trim(), expirationDate: expiration ?? null,
            encrypt, password: encrypt ? password : undefined,
        });
        setSaving(true);
        try {
            await releaseStore.updateItem();
        } finally {
            setSaving(false);
        }
    };

    return <Dialog title="编辑项目发布" open={true} onClose={close} width={450}
                   mask maskClosable={false} centered showClose>
        <div className={'release-edit-panel'} style={{padding: '10px 20px'}}>
            <Grid gridGap={'20px'}>
                <Input label={'名称'} value={name} onChange={setName} disabled={saving}/>
                <NumberInput label={'有效期'} value={expiration} placeholder={'填写数字。单位：天'}
                             onChange={setExpiration} disabled={saving}/>
                <Switch label={'加密'} value={encrypt} onChange={setEncrypt} disabled={saving}/>
                {encrypt && <Input label={'密码'} type="password" value={password}
                                   onChange={setPassword} disabled={saving}/>}
            </Grid>
            <div style={{display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 20}}>
                <Button type="primary" onClick={doUpdate} loading={saving} disabled={saving}>更新</Button>
                <Button type="default" onClick={close} disabled={saving}>取消</Button>
            </div>
        </div>
    </Dialog>;
});

export default ReleaseEditPanel;
