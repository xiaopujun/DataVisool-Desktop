/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import "@/pages/home/project-list/ProjectItem.less";
import {CopyOne, DataServer, Delete, Edit, EditOne, FileText, MapDraw, MoreOne, PreviewOpen, Setting} from "@icon-park/react";
import {type MouseEvent, useEffect, useRef, useState} from "react";
import baseApi from "@/api/engineering/BaseApi.ts";
import {IProjectInfoManager} from "@datavisool/sdk/types";
import {useNavigate} from "react-router-dom";
import {globalMessage} from "@datavisool/ui";
import {Dropdown, DropdownMenuItem} from "@datavisool/ui";
import {tx} from "@/i18n/miniI18n.ts";

export interface ProjectItemProps {
    id: string;
    name: string;
    cover?: string;
    description?: string;
    updatedAt?: string;
    variant?: "analytics" | "mimo" | "map";
    doOperate: (id: string, type: string) => void;
}

export default function ProjectItem(props: ProjectItemProps) {
    const {cover, description, id, updatedAt, variant = "analytics", doOperate} = props;
    const [rename, setRename] = useState(false);
    const [name, setName] = useState(props.name);
    const [draftName, setDraftName] = useState(props.name);
    const savingName = useRef(false);
    const navigate = useNavigate();

    useEffect(() => {
        setName(props.name);
    }, [props.name]);

    const items: DropdownMenuItem[] = [
        {
            key: '1',
            label: '删除',
            icon: <Delete/>,
            danger: true,
            onClick: () => doOperate(id, "del")
        },
        {
            key: '2',
            label: '复制',
            icon: <CopyOne/>,
            onClick: () => doOperate(id, "clone")
        },
        {
            key: '3',
            label: '重命名',
            icon: <Edit/>,
            onClick: () => {
                setDraftName(name);
                setRename(true);
            }
        },
        {
            key: '4',
            label: '编辑',
            icon: <EditOne/>,
            onClick: () => doOperate(id, "edit")
        },
        {
            key: '5',
            label: '预览',
            icon: <PreviewOpen/>,
            onClick: () => doOperate(id, "show")
        },
        {
            key: '6',
            label: '设置',
            icon: <Setting/>,
            onClick: () => navigate(`/home/projectSetting/${id}`, {state: {projectId: id, title: name}})
        }
    ];

    const updateName = () => {
        if (savingName.current) return;
        const nextName = draftName.trim();
        if (!nextName) {
            setDraftName(name);
            setRename(false);
            globalMessage.messageApi?.error(tx("项目名称不能为空"));
            return;
        }
        if (nextName === name) {
            setRename(false);
            return;
        }
        savingName.current = true;
        const data: IProjectInfoManager = {id, name: nextName};
        baseApi.updateProject(data).then((result) => {
            if (result) {
                setName(nextName);
                setRename(false);
                globalMessage.messageApi?.success(tx("保存成功"));
            } else {
                globalMessage.messageApi?.error(tx("保存失败"));
            }
        }).catch(() => {
            globalMessage.messageApi?.error(tx("保存失败"));
        }).finally(() => {
            savingName.current = false;
        });
    }

    const renderProjectIcon = () => {
        if (variant === "mimo") return <DataServer theme="outline" size={28}/>;
        if (variant === "map") return <MapDraw theme="outline" size={28}/>;
        return <FileText theme="outline" size={28}/>;
    }

    const displayDescription = description?.trim() || tx("未填写项目描述");

    const handleDoubleClick = (event: MouseEvent<HTMLDivElement>) => {
        if (rename) {
            return;
        }

        const target = event.target;
        if (target instanceof Element && target.closest("button, input, textarea, select, [role='button'], [data-project-card-ignore-dblclick='true']")) {
            return;
        }

        doOperate(id, "edit");
    }

    return (
        <div className={`project-list-item project-item-${variant}`} onDoubleClick={handleDoubleClick}>
            <div className="project-item-cover" style={{backgroundImage: `url(${cover})`}}>
                <Dropdown menu={{items}} trigger={["click"]} placement="bottomRight">
                    <button className="project-cover-menu" type="button" data-project-card-ignore-dblclick="true" onClick={(event) => event.stopPropagation()}>
                        <MoreOne theme="outline" size={22}/>
                    </button>
                </Dropdown>
            </div>
            <div className="project-item-content">
                <div className="project-item-icon">{renderProjectIcon()}</div>
                <div className="project-item-main">
                    <div className="project-title-row">
                        <div className="project-name">
                            {rename ?
                                <div className="project-rename-input">
                                    <input
                                        autoFocus={true}
                                        value={draftName}
                                        onFocus={event => event.currentTarget.select()}
                                        onBlur={updateName}
                                        onKeyDown={(event) => {
                                            if (event.key === 'Enter') {
                                                updateName();
                                            } else if (event.key === 'Escape') {
                                                setDraftName(name);
                                                setRename(false);
                                            }
                                        }}
                                        onChange={(e) => setDraftName(e.target.value)}
                                    />
                                </div> :
                                <div className="project-name-content" title={name}>{name}</div>
                            }
                        </div>
                        <div className="project-update-time">编辑于 {updatedAt || "暂无时间"}</div>
                    </div>
                    <div className="project-description" title={displayDescription}>
                        {displayDescription}
                    </div>
                </div>
            </div>
            {rename && (
                <div className="project-item-rename-save"
                     onMouseDown={(event) => event.preventDefault()}
                     onClick={() => {
                         updateName();
                     }}>
                    保存
                </div>
            )}
        </div>
    )
}
