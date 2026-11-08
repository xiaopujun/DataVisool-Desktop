/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {CSSProperties, ReactNode, useCallback, useEffect, useRef} from 'react'
import {debounce} from "lodash";
import './ScreenFit.less';
import {AdaptationType} from "@datavisool/sdk/types";

interface ScreenFitProps {
    children?: ReactNode
    width: number | string
    height: number | string,
    bodyOverflowHidden?: boolean
    delay?: number
    mode?: AdaptationType
    scaleChange?: (xScale: number, yScale: number) => void
}

const screenFitStyleMap: Record<string, CSSProperties> = {
    'scale': {overflow: 'hidden'},
    'full-screen': {overflow: 'hidden'},
    'full-x': {overflow: 'hidden scroll'},
    'full-y': {overflow: 'scroll hidden'}
}

export default function ScreenFit(props: ScreenFitProps) {
    const {width, height, bodyOverflowHidden = true, delay = 200, scaleChange} = props
    const boxRef = useRef<HTMLDivElement>(null)
    const elRef = useRef<HTMLDivElement>(null)
    const bodyOverflowRef = useRef<string>('')
    const lastScaleRef = useRef<{ x: number, y: number } | null>(null)

    const getRealSize = useCallback(() => {
        const parsedWidth = Number(width);
        const parsedHeight = Number(height);
        return {
            width: Number.isFinite(parsedWidth) && parsedWidth > 0 ? parsedWidth : window.screen.width,
            height: Number.isFinite(parsedHeight) && parsedHeight > 0 ? parsedHeight : window.screen.height
        }
    }, [height, width])

    const notifyScaleChange = useCallback((xScale: number, yScale: number) => {
        const lastScale = lastScaleRef.current;
        if (lastScale &&
            Math.abs(lastScale.x - xScale) < 0.0001 &&
            Math.abs(lastScale.y - yScale) < 0.0001)
            return;
        lastScaleRef.current = {x: xScale, y: yScale};
        scaleChange && scaleChange(xScale, yScale);
    }, [scaleChange])

    const updateScale = useCallback(() => {
        if (!boxRef.current || !elRef.current) return;
        // 使用组件所在容器的可用尺寸，避免把标题栏等外部区域计入画布
        const currentWidth = boxRef.current.clientWidth
        const currentHeight = boxRef.current.clientHeight
        // 获取大屏最终的宽高
        const {width: realWidth, height: realHeight} = getRealSize()
        // 计算缩放比例
        const widthScale = currentWidth / +realWidth
        const heightScale = currentHeight / +realHeight

        let scale, domWidth, domHeight, mx, my;
        elRef.current.style.transformOrigin = 'left top'
        elRef.current.style.margin = '0'
        elRef.current.style.marginBottom = '0'
        elRef.current.style.marginRight = '0'
        elRef.current.style.paddingBottom = '0'
        elRef.current.style.paddingRight = '0'
        switch (props.mode) {
            case 'full-screen':
                // 若要铺满全屏，则按照各自比例缩放
                elRef.current!.style.transform = `scale(${widthScale},${heightScale})`
                notifyScaleChange(widthScale, heightScale)
                break;
            case 'full-x': {
                // 宽度铺满，高度按相同比例缩放（保持宽高比）
                elRef.current!.style.transform = `scale(${widthScale}, ${widthScale})`
                elRef.current!.style.transformOrigin = 'left top'
                elRef.current!.style.margin = '0'

                // 关键修复：计算缩放后的实际显示高度
                const scaledHeight = +realHeight * widthScale

                // 使用 padding-bottom 撑开父容器，消除 transform 导致的布局空白
                // padding-bottom 会被计入布局高度，解决滚动条问题
                const heightDiff = +realHeight - scaledHeight
                if (heightDiff > 0) {
                    // 需要减去差值，因为 transform 缩小后占用空间变小了
                    elRef.current!.style.paddingBottom = `0px`
                    elRef.current!.style.marginBottom = `-${heightDiff}px`
                } else {
                    elRef.current!.style.paddingBottom = `0px`
                    elRef.current!.style.marginBottom = `0px`
                }

                notifyScaleChange(widthScale, widthScale)
                break;
            }
            case 'full-y': {
                // 高度铺满，宽度按相同比例缩放（保持宽高比）
                elRef.current!.style.transform = `scale(${heightScale}, ${heightScale})`
                elRef.current!.style.transformOrigin = 'left top'
                elRef.current!.style.margin = '0'

                // 同样的逻辑应用到宽度
                const scaledWidth = +realWidth * heightScale
                const widthDiff = +realWidth - scaledWidth
                if (widthDiff > 0) {
                    elRef.current!.style.paddingRight = `0px`
                    elRef.current!.style.marginRight = `-${widthDiff}px`
                } else {
                    elRef.current!.style.paddingRight = `0px`
                    elRef.current!.style.marginRight = `0px`
                }

                notifyScaleChange(heightScale, heightScale)
                break;
            }
            default:
                // 按照宽高最小比例进行缩放
                scale = Math.min(widthScale, heightScale)
                elRef.current!.style.transform = `scale(${scale},${scale})`;
                domWidth = elRef.current!.clientWidth
                domHeight = elRef.current!.clientHeight
                mx = Math.max((currentWidth - domWidth * scale) / 2, 0)
                my = Math.max((currentHeight - domHeight * scale) / 2, 0)
                elRef.current!.style.margin = `${my}px ${mx}px`
                elRef.current!.style.marginBottom = `${my}px`
                notifyScaleChange(scale, scale)
                break;
        }
    }, [getRealSize, notifyScaleChange, props.mode])

    const updateSize = useCallback(() => {
        if (!elRef.current) return;
        const {width: realWidth, height: realHeight} = getRealSize();
        elRef.current.style.width = `${realWidth}px`
        elRef.current.style.height = `${realHeight}px`
    }, [getRealSize])

    useEffect(() => {
        if (bodyOverflowHidden) {
            bodyOverflowRef.current = document.body.style.overflow
            document.body.style.overflow = 'hidden'
        }
        const onResize = debounce(() => {
            updateSize()
            updateScale()
        }, delay);
        updateSize()
        updateScale()
        const resizeObserver = new ResizeObserver(() => onResize())
        resizeObserver.observe(boxRef.current!)
        window.addEventListener('resize', onResize);
        return () => {
            resizeObserver.disconnect()
            window.removeEventListener('resize', onResize);
            if (bodyOverflowHidden)
                document.body.style.overflow = bodyOverflowRef.current;
            onResize.cancel?.();
        }
    }, [bodyOverflowHidden, delay, updateScale, updateSize])

    const scrollStyle = screenFitStyleMap[props.mode || 'scale']

    return (
        <div style={{...scrollStyle}}
             ref={boxRef}
             className={'react-screen-box'}>
            <div className={'screen-wrapper'}
                 ref={elRef}>
                {props.children}
            </div>
        </div>
    )
}
