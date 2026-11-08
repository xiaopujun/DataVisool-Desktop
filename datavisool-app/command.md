## 辅助工具

```ts
// 快速渲染当前图层链表结构关系
window.renderLinks = () => {
    const links = Object.values(window.DATAVISOOL_ENV.currentManager.layerManager.layerConfigs).map(i => {
        const layers = window.DATAVISOOL_ENV.currentManager.layerManager.layerConfigs;
        return {id: i.id, name: i.name, pid: layers[i.pid]?.name, prev: layers[i.prev]?.name, next: layers[i.next]?.name}
    })
    console.log(links)
}
```
