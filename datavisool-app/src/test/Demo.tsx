/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import './DemoMain.less';
import {useMemo, useState} from "react";
import {Api, CopyOne, DashboardOne, DatabaseSetting, DataOne, Delete, Edit, MoreOne, PreviewOpen, Send, Setting, Upload as UploadIcon} from "@icon-park/react";
import {
    Accordion,
    Avatar,
    Button,
    CardPanel,
    CheckBox,
    CodeEditor,
    ColorMode,
    ColorPicker,
    ColorsPicker,
    ControlGroup,
    Dialog,
    Dropdown,
    Empty,
    FrameLayout,
    FullEditor,
    GlobalMessage,
    GlobalModal,
    Grid,
    GroupButton,
    ImageUpload,
    Input,
    DatavisoolGUI,
    DatavisoolGUIUtil,
    Loading,
    Menu,
    MonacoEditor,
    NumberInput,
    Pagination,
    Progress,
    Radio,
    RangeSlider,
    SearchableSelect,
    Select,
    Slider,
    SubAccordion,
    Switch,
    TextArea,
    TextOnly,
    UIContainer,
    Upload,
    globalMessage,
    globalModal,
    type Control,
    type DropdownMenuItem,
    type FieldChangeData,
    type IImageData,
    type MenuItemType,
    type UploadFile,
} from "@datavisool/ui";

const selectOptions = [
    {label: '项目看板', value: 'dashboard'},
    {label: '数据大屏', value: 'screen'},
    {label: '资源管理', value: 'resource'},
];

const searchData = [
    {id: 'api', name: '接口管理'},
    {id: 'font', name: '字体资源'},
    {id: 'video', name: '视频资源'},
    {id: 'model', name: '三维模型'},
];

const sampleImage = 'data:image/svg+xml;utf8,' + encodeURIComponent(`
<svg xmlns="http://www.w3.org/2000/svg" width="520" height="260" viewBox="0 0 520 260">
  <rect width="520" height="260" rx="28" fill="#07111f"/>
  <circle cx="418" cy="58" r="72" fill="#315cff" opacity=".42"/>
  <circle cx="126" cy="192" r="88" fill="#ec4899" opacity=".32"/>
  <path d="M66 176 C136 78 205 98 263 142 S382 187 454 102" fill="none" stroke="#38bdf8" stroke-width="12" stroke-linecap="round"/>
  <rect x="70" y="54" width="202" height="28" rx="14" fill="#ffffff" opacity=".86"/>
  <rect x="70" y="96" width="132" height="16" rx="8" fill="#a8b3cf" opacity=".76"/>
</svg>`);

const datavisoolGuiSchema: Control = {
    type: 'card-panel',
    label: 'DatavisoolGUI Schema 面板',
    children: [
        {type: 'input', key: 'title', label: '标题', value: '年度营收趋势'},
        {type: 'number-input', key: 'limit', label: '数据条数', value: 12, config: {min: 1, max: 50}},
        {type: 'switch', key: 'visible', label: '显示图例', value: true},
        {
            type: 'select',
            key: 'theme',
            label: '主题',
            value: 'screen',
            config: {options: selectOptions},
        },
    ],
};

const seriesTemplate: Control = {
    children: [
        {type: 'input', key: 'name', label: '系列名', value: '新增用户'},
        {type: 'color-picker', key: 'color', label: '颜色', value: '#3b82ff', config: {showText: true}},
        {type: 'slider', key: 'opacity', label: '透明度', value: 72, config: {min: 0, max: 100}},
    ],
};

const fetchSearchOptions = async (keywords = '') => {
    const normalized = keywords.trim().toLowerCase();
    return searchData.filter((item) => !normalized || item.name.toLowerCase().includes(normalized));
};

const transformSearchOption = (item: typeof searchData[number]) => ({label: item.name, value: item.id});

function DemoSection(props: { title: string; description: string; children: React.ReactNode }) {
    return (
        <section className="demo-section">
            <div className="demo-section-header">
                <h2>{props.title}</h2>
                <p>{props.description}</p>
            </div>
            {props.children}
        </section>
    );
}

function DemoTile(props: { name: string; children: React.ReactNode }) {
    return (
        <div className="demo-tile">
            <div className="demo-tile-title">{props.name}</div>
            <div className="demo-tile-body">{props.children}</div>
        </div>
    );
}

export default function Demo() {
    const [lastChange, setLastChange] = useState('等待交互');
    const [showFullEditor, setShowFullEditor] = useState(false);
    const [schemaData, setSchemaData] = useState<object>(() => DatavisoolGUIUtil.parseSchemaData(datavisoolGuiSchema));
    const [uploaded, setUploaded] = useState('内置示例图');
    const [demoUploadFiles, setDemoUploadFiles] = useState<UploadFile[]>([]);
    const [demoProgress, setDemoProgress] = useState(48);
    const [demoMenuKey, setDemoMenuKey] = useState('console');
    const [demoMenuOpenKeys, setDemoMenuOpenKeys] = useState(['datasource']);

    const uploadHandle = useMemo(() => async (file: File): Promise<IImageData> => {
        const url = URL.createObjectURL(file);
        setUploaded(file.name);
        return {name: file.name, url};
    }, []);

    const handleFieldChange = (change: FieldChangeData) => {
        setSchemaData((prev) => ({...prev, ...change.dataFragment}));
        setLastChange(`${change.dataKeyPath.join('.')} = ${JSON.stringify(change.data)}`);
    };

    const projectDropdownItems = useMemo<DropdownMenuItem[]>(() => [
        {key: 'preview', label: '预览', icon: <PreviewOpen/>, onClick: () => setLastChange('Dropdown = 预览')},
        {key: 'edit', label: '编辑', icon: <Edit/>, onClick: () => setLastChange('Dropdown = 编辑')},
        {key: 'clone', label: '复制', icon: <CopyOne/>, onClick: () => setLastChange('Dropdown = 复制')},
        {key: 'settings', label: '设置', icon: <Setting/>, onClick: () => setLastChange('Dropdown = 设置')},
        {type: 'divider'},
        {key: 'delete', label: '删除', icon: <Delete/>, danger: true, onClick: () => setLastChange('Dropdown = 删除')},
    ], []);

    const projectMenuItems = useMemo<MenuItemType[]>(() => [
        {key: 'console', label: '控制台', icon: <DashboardOne size={16}/>},
        {
            key: 'datasource',
            label: '数据源管理',
            icon: <DataOne size={16}/>,
            children: [
                {key: 'database', label: '数据库管理', icon: <DatabaseSetting size={16}/>},
                {key: 'api', label: 'API 管理', icon: <Api size={16}/>},
            ],
        },
        {key: 'release', label: '发布管理', icon: <Send size={16}/>},
        {type: 'divider'},
        {key: 'setting', label: '设置', icon: <Setting size={16}/>},
    ], []);

    console.log('ddd')
    return (
        <div className="demo-page">
            <GlobalMessage/>
            <GlobalModal/>
            <Dialog open={true} title={'测试标题'} onClose={() => console.log('dd')}>
                <div>dd</div>
            </Dialog>
            <header className="demo-hero">
                <div>
                    <p className="demo-eyebrow">datavisool-ui component lab</p>
                    <h1>组件总览</h1>
                    <p className="demo-subtitle">覆盖当前 `datavisool-ui` 导出的 React
                        组件，保留默认视觉，并给每个组件一个可交互状态。</p>
                </div>
                <div className="demo-status">
                    <span>最后交互</span>
                    <strong>{lastChange}</strong>
                </div>
            </header>

            <DemoSection title="基础输入" description="按钮、文本、数字、开关、单选、多选与滑块。">
                <div className="demo-grid">
                    <DemoTile name="Button">
                        <div className="demo-row">
                            <Button onClick={() => setLastChange('Button primary')}>主按钮</Button>
                            <Button type="default" onClick={() => setLastChange('Button outline')}>次按钮</Button>
                            <Button type="dashed" onClick={() => setLastChange('Button dashed')}>虚线按钮</Button>
                            <Button type="text" onClick={() => setLastChange('Button text')}>文本按钮</Button>
                        </div>
                    </DemoTile>
                    <DemoTile name="Avatar">
                        <div className="demo-avatar-row">
                            <Avatar size="sm" src={sampleImage} alt="avatar" />
                            <Avatar>LC</Avatar>
                            <Avatar size="lg" shape="square">UI</Avatar>
                            <Avatar size={72} src={sampleImage} alt="avatar" />
                        </div>
                    </DemoTile>
                    <DemoTile name="Dropdown">
                        <div className="demo-row">
                            <Dropdown items={projectDropdownItems} trigger={['click']} placement="bottomRight">
                                <Button type="default" icon={<MoreOne/>}>项目操作</Button>
                            </Dropdown>
                            <Dropdown
                                items={projectDropdownItems}
                                trigger={['hover']}
                                placement="bottomLeft"
                            >
                                <Button type="text" icon={<MoreOne/>}>悬浮菜单</Button>
                            </Dropdown>
                        </div>
                    </DemoTile>
                    <DemoTile name="Menu">
                        <div className="demo-menu-shell">
                            <Menu
                                items={projectMenuItems}
                                selectedKeys={[demoMenuKey]}
                                openKeys={demoMenuOpenKeys}
                                onOpenChange={(keys) => setDemoMenuOpenKeys(keys)}
                                onClick={({key}) => {
                                    setDemoMenuKey(key);
                                    setLastChange(`Menu = ${key}`);
                                }}
                            />
                        </div>
                    </DemoTile>
                    <DemoTile name="Input / NumberInput / TextArea / TextOnly">
                        <Input label="Input" defaultValue="DataVisool"
                               onChange={(value) => setLastChange(`Input = ${value}`)}/>
                        <NumberInput label="NumberInput" defaultValue={60} min={0} max={100}
                                     onChange={(value) => setLastChange(`NumberInput = ${value}`)}/>
                        <TextArea label="TextArea" defaultValue="这里可以编辑多行配置说明。"
                                  onChange={(value) => setLastChange(`TextArea length = ${value.length}`)}/>
                        <TextOnly label="TextOnly" defaultValue="只读说明文本，用于配置摘要。"/>
                    </DemoTile>
                    <DemoTile name="CheckBox / Switch / Radio">
                        <CheckBox label="CheckBox" defaultValue
                                  onChange={(value) => setLastChange(`CheckBox = ${value}`)}/>
                        <Switch label="Switch" defaultValue onChange={(value) => setLastChange(`Switch = ${value}`)}/>
                        <Radio
                            label="Radio"
                            defaultValue="design"
                            options={[
                                {label: '设计', value: 'design'},
                                {label: '预览', value: 'preview'},
                                {label: '发布', value: 'release'},
                            ]}
                            onChange={(value) => setLastChange(`Radio = ${value}`)}
                        />
                    </DemoTile>
                    <DemoTile name="Slider / RangeSlider / GroupButton">
                        <Slider label="Slider" defaultValue={42} min={0} max={100}
                                onChange={(value) => console.log(value)}/>
                        <RangeSlider label="RangeSlider" defaultValue={[24, 78]} min={0} max={100}
                                     onChange={(value) => setLastChange(`RangeSlider = ${value.join(',')}`)}/>
                        <GroupButton
                            label="GroupButton"
                            defaultValue="canvas"
                            items={[
                                {value: 'canvas', content: '画布'},
                                {value: 'data', content: '数据'},
                                {value: 'style', content: '样式'},
                            ]}
                            onChange={(value) => setLastChange(`GroupButton = ${value}`)}
                        />
                    </DemoTile>
                    <DemoTile name="Pagination">
                        <Pagination
                            label="Pagination"
                            total={248}
                            defaultCurrent={3}
                            defaultPageSize={20}
                            showTotal={(total, range) => `第 ${range[0]}-${range[1]} 条 / 共 ${total} 条`}
                            showSizeChanger
                            showQuickJumper
                            pageSizeOptions={[10, 20, 50]}
                            onChange={(page, pageSize) => setLastChange(`Pagination = page ${page}, pageSize ${pageSize}`)}
                            onPageSizeChange={(pageSize, page) => setLastChange(`Pagination pageSize = ${pageSize}, page ${page}`)}
                        />
                    </DemoTile>
                </div>
            </DemoSection>

            <DemoSection title="选择与颜色" description="普通选择、搜索选择、单色、多色和颜色模式组合。">
                <div className="demo-grid">
                    <DemoTile name="Select">
                        <Select label="单选 Select" defaultValue="dashboard" options={selectOptions}
                                onChange={(value) => setLastChange(`Select = ${value}`)}/>
                        <Select label="多选 Select" defaultValue={['dashboard', 'resource']} options={selectOptions}
                                onChange={(value) => setLastChange(`MultiSelect = ${JSON.stringify(value)}`)}/>
                        <Select label="可搜索 Select" showSearch placeholder="搜索模块" options={selectOptions}
                                onChange={(value) => setLastChange(`Search Select = ${value}`)}/>
                    </DemoTile>
                    <DemoTile name="SearchableSelect">
                        <SearchableSelect
                            defaultValue="api"
                            fetcher={fetchSearchOptions}
                            transformer={transformSearchOption}
                            initializer={async () => searchData}
                            onChange={(value) => setLastChange(`SearchableSelect = ${value}`)}
                        />
                    </DemoTile>
                    <DemoTile name="ColorPicker / ColorsPicker / ColorMode">
                        <ColorPicker label="ColorPicker" defaultValue="#38bdf8" showText
                                     onChange={(value) => setLastChange(`ColorPicker = ${value}`)}/>
                        <ColorsPicker label="ColorsPicker" defaultValue={['#3b82ff', '#ec4899', '#22d3a6']} canAdd
                                      onChange={(value) => setLastChange(`ColorsPicker = ${value.join(',')}`)}/>
                        <ColorMode label="ColorMode" defaultValue={['#315cff', '#b53cff']}
                                   onChange={(value) => setLastChange(`ColorMode = ${JSON.stringify(value)}`)}/>
                    </DemoTile>
                </div>
            </DemoSection>

            <DemoSection title="布局容器" description="容器、网格、面板、折叠面板和框架布局。">
                <div className="demo-grid">
                    <DemoTile name="UIContainer / Grid">
                        <UIContainer label="UIContainer" tip="容器提示">
                            <div className="demo-inline-surface">容器内容区域</div>
                        </UIContainer>
                        <Grid label="Grid" columns={3} gridGap="10px">
                            <div className="demo-mini-cell">A</div>
                            <div className="demo-mini-cell">B</div>
                            <div className="demo-mini-cell">C</div>
                        </Grid>
                    </DemoTile>
                    <DemoTile name="CardPanel / Accordion / SubAccordion">
                        <CardPanel label="CardPanel" tip="标准面板">
                            <p className="demo-muted">面板内可以承载任意内容。</p>
                        </CardPanel>
                        <Accordion label="Accordion" defaultActive>
                            <p className="demo-muted">展开后显示的内容。</p>
                        </Accordion>
                        <SubAccordion label="SubAccordion" defaultValue>
                            <p className="demo-muted">子折叠面板复用 Accordion 行为。</p>
                        </SubAccordion>
                    </DemoTile>
                    <DemoTile name="FrameLayout">
                        <div className="demo-frame-wrap">
                            <FrameLayout
                                header={<div className="demo-frame-bar">Header</div>}
                                left={<div className="demo-frame-side">Left</div>}
                                right={<div className="demo-frame-side">Right</div>}
                                content={<div className="demo-frame-content">Content</div>}
                                footer={<div className="demo-frame-bar">Footer</div>}
                            />
                        </div>
                    </DemoTile>
                </div>
            </DemoSection>

            <DemoSection title="媒体与编辑器" description="图片上传、轻量 Monaco 替身、代码编辑器和全屏编辑器。">
                <div className="demo-grid">
                    <DemoTile name="ImageUpload">
                        <ImageUpload
                            label={`ImageUpload：${uploaded}`}
                            defaultValue={sampleImage}
                            accept="image/*"
                            size={5}
                            uploadHandle={uploadHandle}
                            onChange={(value) => setLastChange(`ImageUpload = ${value.url ? 'selected' : 'cleared'}`)}
                        />
                    </DemoTile>
                    <DemoTile name="Upload.Dragger / Progress">
                        <Upload.Dragger
                            multiple
                            maxCount={4}
                            fileList={demoUploadFiles}
                            beforeUpload={(file) => {
                                const nextFile: UploadFile = {
                                    uid: `${file.name}-${file.lastModified}-${file.size}`,
                                    name: file.name,
                                    status: 'done',
                                    originFileObj: file,
                                };
                                setDemoUploadFiles((prev) => [...prev, nextFile].slice(0, 4));
                                setLastChange(`Upload.Dragger = ${file.name}`);
                                return false;
                            }}
                            onRemove={(file) => {
                                setDemoUploadFiles((prev) => prev.filter((item) => item.uid !== file.uid));
                                setLastChange(`Upload remove = ${file.name}`);
                            }}
                        >
                            <UploadIcon fill="#717171" size={22}/>
                            <p className="demo-upload-hint">点击或拖拽文件到此处</p>
                        </Upload.Dragger>
                        <div className="demo-progress-stack">
                            <Progress percent={demoProgress} status="active"/>
                            <Progress percent={100} status="success"/>
                            <Progress percent={32} status="exception"/>
                        </div>
                        <Slider
                            label="调整 Progress"
                            min={0}
                            max={100}
                            value={demoProgress}
                            onChange={(value) => {
                                setDemoProgress(value);
                                setLastChange(`Progress = ${value}%`);
                            }}
                        />
                    </DemoTile>
                    <DemoTile name="MonacoEditor / CodeEditor">
                        <MonacoEditor
                            label="MonacoEditor"
                            language="json"
                            defaultValue={'{\n  "theme": "dark",\n  "density": "compact"\n}'}
                            height={150}
                            onChange={(value) => setLastChange(`MonacoEditor length = ${value?.length ?? 0}`)}
                        />
                        <CodeEditor
                            label="CodeEditor"
                            fullScreen
                            language="javascript"
                            defaultValue={'export const answer = 42;'}
                            height={130}
                            onChange={(value) => setLastChange(`CodeEditor length = ${value?.length ?? 0}`)}
                        />
                    </DemoTile>
                    <DemoTile name="FullEditor">
                        <Button onClick={() => setShowFullEditor(true)}>打开 FullEditor</Button>
                        <p className="demo-muted">点击按钮打开覆盖层，保存后会回写交互状态。</p>
                        {showFullEditor ? (
                            <FullEditor
                                language="sql"
                                defaultValue={'select * from project where status = "active";'}
                                onChange={(value) => setLastChange(`FullEditor saved = ${value?.slice(0, 24)}`)}
                                onClose={() => setShowFullEditor(false)}
                            />
                        ) : null}
                    </DemoTile>
                </div>
            </DemoSection>

            <DemoSection title="反馈与 Schema" description="全局消息、系统弹窗、Loading、DatavisoolGUI 和动态 ControlGroup。">
                <div className="demo-grid">
                    <DemoTile name="GlobalMessage / GlobalModal / Loading">
                        <div className="demo-row">
                            <Button onClick={() => globalMessage.success('操作成功')}>Success</Button>
                            <Button type="default"
                                    onClick={() => globalMessage.warning('注意当前配置')}>Warning</Button>
                            <Button type="default" onClick={() => globalModal.modalApi.info({
                                title: 'GlobalModal',
                                content: '这是 datavisool-ui 的全局弹窗效果。'
                            })}>Modal</Button>
                        </div>
                        <div className="demo-loading-box">
                            <Loading height={92}/>
                        </div>
                    </DemoTile>
                    <DemoTile name="Empty">
                        <Empty
                            variant="panel"
                            title="暂无资源"
                            description="创建或导入资源后会显示在这里。"
                            action={<Button size="sm" onClick={() => setLastChange('Empty action')}>新建资源</Button>}
                        />
                    </DemoTile>
                    <DemoTile name="DatavisoolGUI">
                        <DatavisoolGUI schema={datavisoolGuiSchema} onFieldChange={handleFieldChange}/>
                        <pre className="demo-code">{JSON.stringify(schemaData, null, 2)}</pre>
                    </DemoTile>
                    <DemoTile name="ControlGroup">
                        <ControlGroup
                            label="ControlGroup"
                            itemName="数据系列"
                            template={seriesTemplate}
                            defaultValue={[
                                {name: '新增用户', color: '#3b82ff', opacity: 72},
                                {name: '活跃用户', color: '#22d3a6', opacity: 64},
                            ]}
                            onChange={(value) => setLastChange(`ControlGroup = ${JSON.stringify(value)}`)}
                        />
                    </DemoTile>
                </div>
            </DemoSection>
        </div>
    );
}
