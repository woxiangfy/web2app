/**
 * Polyfill for WebView2 Drag and Drop Bug
 *
 * This polyfill patches a known bug in WebView2 where native (HTML5) drag & drop
 * is unreliable: the WebView2 host intercepts the drag gesture through Windows OLE
 * and swallows the page-side `dragstart/dragover/drop` events. As a result, inside
 * the embedded page a `[draggable]` element can neither be dropped onto a target
 * (host steals it) nor reliably clicked.
 *
 * This script re-implements the whole drag gesture with plain mouse events and
 * synthesizes the full HTML5 drag-and-drop event sequence the page expects:
 *
 *     mousedown ─┬─(no movement)────────────────────────────► native `click`
 *                │
 *                └─(pointer moved > threshold)─► dragstart
 *                                                ┌ dragenter (enter a new target)
 *                                                ├ dragover  (continuously, while hovering a target)
 *                                                ├ dragleave (when leaving a target)
 *                                                └ drop ─► dragend  (on mouseup)
 *
 * Why a full rewrite of the old logic was needed (real-world pages still failed):
 *   1. clientX/clientY were passed inside the `new Event(type, options)` init dict,
 *      but the plain `EventInit` type does NOT accept coordinates, so they ended up
 *      `undefined`. Drag/drop libraries that compute the insertion point from
 *      `e.clientX/e.clientY` therefore could not place the item anywhere.
 *      → We now build a real `new DragEvent(...)` (Chromium supports it), whose init
 *        natively carries `clientX/clientY`/`dataTransfer`, with a manual fallback.
 *   2. Only `dragover` was fired, and only when the target *changed* — never
 *      `dragenter`/`dragleave`. Libraries drive their "highlight/dropzone active"
 *      state off dragenter/dragleave, so a target never became droppable.
 *      → We now fire dragenter on entering, dragleave on leaving, and dragover
 *        continuously while hovering, matching native browser behavior.
 *   3. The visual ghost overlay was an element under the cursor that could be
 *      returned by `elementFromPoint`, so we kept missing the real target below.
 *      → The ghost now gets `pointer-events:none`.
 *   4. Each synthetic event used a fresh empty mock DataTransfer, so data written
 *      by the page's dragstart handler was lost by drop time.
 *      → The whole gesture shares one DataTransfer instance (native when available).
 *
 * Copyright (c) 2023 Iain Fraser (MIT). Heavily reworked for this project.
 */
(function (opts) {
    'use strict';

    // 判定“拖动”的最小位移（像素）。小于它视为一次点击，保证 click 正常。
    const DRAG_THRESHOLD = (opts && opts.dragThreshold) || 4;

    let isDragging = false;        // 已判定为拖动（位移超阈值、已发 dragstart）
    let pendingElement = null;     // 刚按下、尚未判定拖/点的 draggable 候选
    let draggedElement = null;     // 真正被拖动、且已派发过 dragstart 的元素
    let startPosition = { x: 0, y: 0 };
    let currentOverElement = null; // 当前悬停目标（已发 dragenter 的）
    let ghostElement = null;
    let sessionDataTransfer = null;// 一次拖拽会话共享的 DataTransfer

    /* ------------------------------------------------------------------ *
     *  DataTransfer：优先用 Chromium 原生构造函数（WebView2 是 Chromium）。
     *  原生实例的 `types` 是只读 DOMStringList、`setData/getData` 对 format
     *  做了小写规范化，拖放库能正确识别；自造 mock 仅是兜底。
     * ------------------------------------------------------------------ */
    const NativeDataTransfer = (function () {
        try {
            var t = new DataTransfer();
            // 原生对象上 setData 后 types 必须能反映出来，否则退回 mock。
            t.setData('text/plain', 'probe');
            return typeof t.getData === 'function' && t.types.length >= 1 ? DataTransfer : null;
        } catch (e) {
            return null;
        }
    })();

    function makeDataTransfer() {
        if (NativeDataTransfer) {
            try { return new NativeDataTransfer(); } catch (e) { /* fallthrough */ }
        }
        return new MockDataTransfer();
    }

    function MockDataTransfer() {
        this.dropEffect = 'move';
        this.effectAllowed = 'all';
        this.files = [];
        this.items = [];
        this.types = [];
        this.setData = function (format, data) {
            this.items = this.items.filter(function (i) { return i.format !== format; });
            this.items.push({ format: format, data: String(data) });
            this.types = this.items.map(function (i) { return i.format; });
        };
        this.getData = function (format) {
            var item = this.items.find(function (i) { return i.format === format; });
            return item ? item.data : '';
        };
        this.clearData = function (format) {
            this.items = this.items.filter(function (i) { return i.format !== format; });
            this.types = this.items.map(function (i) { return i.format; });
        };
    }

    /* ------------------------------------------------------------------ *
     *  合成拖拽事件。优先 `new DragEvent(type, init)` —— Chromium 的
     *  DragEventInit 原生支持 clientX/clientY/screenX/screenY/dataTransfer，
     *  这样坐标能真正落到事件对象上（旧实现把坐标塞进 EventInit 导致
     *  undefined)。极旧环境若无 DragEvent 再退化为 Event + 手动补坐标。
     *
     *  关键修补：原生拖拽事件的 pageX/pageY = clientX/clientY + 页面滚动偏移。
     *  但【合成】DragoEvent 的 pageX/pageY 由 Chromium 从 clientX/clientY 推导，
     *  【不含滚动偏移】。而 Trilium 的树（jquery.fancytree + dnd5 扩展）计算插入
     *  位置正是用 `event.pageY` 对照目标行的文档坐标 `$target.offset().top`：
     *
     *      relPosY = (event.pageY - $target.offset().top) / $target.height();
     *      // relPosY<=0.25 插到该行前；0.25~0.75 作为子节点；>=0.75 插到该行后
     *
     *  一旦树或窗口发生滚动，clientY 与 pageY 便不一致；若 pageY 缺了滚动偏移，
     *  relPosY 就按错的行内相对位置计算 → hitMode 判定错误 → 重排到错误位置或失效。
     *  因此这里强制给每个合成事件补齐 pageX/pageY（用手写属性遮蔽只读 getter，
     *  strict 模式下用 defineProperty 避免对 getter-only 原型属性赋值抛错）。
     * ------------------------------------------------------------------ */
    function buildDragEvent(type, x, y) {
        var dt = sessionDataTransfer || makeDataTransfer();

        // 会话 DataTransfer 需要在事件构造时就注入（dataTransfer 只能由浏览器
        // 或 DragEventInit 提供；事后给 Event 赋 dataTransfer 属性在多数库仍可读，
        // 但用原生 DragEvent 更稳妥）。因此这里传一个占位 dt。
        var ev;
        if (typeof DragEvent === 'function') {
            try {
                ev = new DragEvent(type, {
                    bubbles: true, cancelable: true, view: window,
                    dataTransfer: dt,
                    clientX: x, clientY: y,
                    screenX: (x || 0), screenY: (y || 0)
                });
            } catch (e) { ev = null; } // 构造失败则回退到下面手动补坐标
        }
        if (!ev) {
            ev = new Event(type, { bubbles: true, cancelable: true });
            if (x !== undefined) ev.clientX = x;
            if (y !== undefined) ev.clientY = y;
            ev.screenX = x || 0;
            ev.screenY = y || 0;
            ev.dataTransfer = dt;
        }
        // 兜底确保 dataTransfer 一定存在（原生 DragEvent 可能重建出新的空实例）
        if (!ev.dataTransfer) ev.dataTransfer = dt;

        // 补齐 pageX/pageY（含滚动偏移，与真实拖拽事件一致）。
        // pageX/pageY 在原型上是只读 getter，直接赋值在 strict 模式会抛错，
        // 用 defineProperty 造一个遮蔽它的实例自有数据属性。
        var sx = window.pageXOffset || document.documentElement.scrollLeft || 0;
        var sy = window.pageYOffset || document.documentElement.scrollTop || 0;
        try {
            Object.defineProperty(ev, 'pageX', {
                value: (x === undefined ? sx : x + sx),
                configurable: true, writable: true
            });
            Object.defineProperty(ev, 'pageY', {
                value: (y === undefined ? sy : y + sy),
                configurable: true, writable: true
            });
        } catch (e2) { /* 个别环境不可覆盖时忽略，保留客户端坐标 */ }
        return ev;
    }

    /** 找出最近的 draggable 祖先。
     *  兼容两种写法：`draggable="true"` 内容属性，或 DOM 属性 `el.draggable = true`
     *  （fancytree 等库用属性赋值，属性虽会反射回内容属性，这里双保险更稳妥）。 */
    function isDraggable(element) {
        if (element.getAttribute && element.getAttribute('draggable') === 'true') return true;
        try { return element.draggable === true; } catch (e) { return false; }
    }
    function findDraggableAncestor(element) {
        while (element && element !== document.body && element.nodeType === 1) {
            if (isDraggable(element)) {
                return element;
            }
            element = element.parentElement;
        }
        return null;
    }

    /* ------------------------------------------------------------------ *
     *  Ghost（视觉拖动副本）
     * ------------------------------------------------------------------ */
    function applyStylesToGhost(original, ghost) {
        var computedStyles;
        try { computedStyles = window.getComputedStyle(original); }
        catch (e) { computedStyles = null; }
        if (computedStyles) {
            for (var i = 0; i < computedStyles.length; i++) {
                var prop = computedStyles[i];
                // 有些只读/特殊样式直接写会抛异常或导致 ghost 尺寸异常，跳过。
                try {
                    var val = computedStyles.getPropertyValue(prop);
                    if (prop === 'position') continue;      // 稍后强制 fixed
                    if (prop === 'display' && val === 'none') continue; // 父隐藏项不该被拖
                    ghost.style.setProperty(prop, val);
                } catch (e2) { /* skip problematic property */ }
            }
        }

        var rect = original.getBoundingClientRect();
        ghost.style.position = 'fixed';
        ghost.style.left = rect.left + 'px';
        ghost.style.top = rect.top + 'px';
        ghost.style.margin = '0';
        ghost.style.zIndex = '2147483000';
        ghost.style.opacity = '0.7';
        // 关键：ghost 不能成为 elementFromPoint 的命中目标，否则探测不到下方元素。
        ghost.style.pointerEvents = 'none';
        // 移除可能干扰 transform 的定位属性
        ghost.style.transform = 'translate(0px, 0px)';

        // 去掉可拖属性，避免 ghost 里出现可交互控件触发二次拖拽
        var all = ghost.querySelectorAll('[draggable="true"]');
        for (var j = 0; j < all.length; j++) all[j].setAttribute('draggable', 'false');

        document.body.appendChild(ghost);
    }

    function clearGhost() {
        if (ghostElement && ghostElement.parentNode) {
            ghostElement.parentNode.removeChild(ghostElement);
        }
        ghostElement = null;
    }

    /* ------------------------------------------------------------------ *
     *  目标探测：返回指针正下方（排除 ghost）的最深元素。
     *  ghost 已 pointer-events:none，无需再 display:none 切换。
     * ------------------------------------------------------------------ */
    function elementBelow(x, y) {
        if (typeof document.elementFromPoint === 'function') {
            try { return document.elementFromPoint(x, y); } catch (e) { return null; }
        }
        return null;
    }

    /** 一次拖动内，指针移到一个“新的悬停目标”时发 dragenter（旧目标发 dragleave）。
     *  移到空白（无目标）处时，给旧目标补一个 dragleave 并清空悬停，符合原生行为。 */
    function updateHover(x, y) {
        var el = elementBelow(x, y);
        if (!el) {
            // 指针已离开所有可放区域：让旧目标收到 dragleave，避免 UI 卡在高亮态。
            if (currentOverElement) {
                fireOn(currentOverElement, 'dragleave', x, y);
                currentOverElement = null;
            }
            return;
        }
        if (el === currentOverElement) {
            // 仍在同一目标上：连续 dragover（让库每帧刷新插入位置 / 高亮）
            fireOn(el, 'dragover', x, y);
            return;
        }
        // 进入新目标：先 leave 旧的，再 enter 新的，再 dragover
        if (currentOverElement) {
            fireOn(currentOverElement, 'dragleave', x, y);
        }
        currentOverElement = el;
        fireOn(el, 'dragenter', x, y);
        fireOn(el, 'dragover', x, y);
    }

    function fireOn(target, type, x, y) {
        if (!target || !target.dispatchEvent) return;
        try {
            target.dispatchEvent(buildDragEvent(type, x, y));
        } catch (e) { /* ignore per-target failures */ }
    }

    /* ------------------------------------------------------------------ *
     *  真正开始拖动：创建 ghost、建立会话 DataTransfer、发 dragstart。
     * ------------------------------------------------------------------ */
    function beginDrag(element, clientX, clientY) {
        draggedElement = element;

        // 会话共享 DataTransfer：dragstart 里页面 setData 的数据，
        // 要在之后的 dragover/drop 里能 getData 回来。
        sessionDataTransfer = makeDataTransfer();

        // 视觉 ghost
        try {
            ghostElement = element.cloneNode(true);
            applyStylesToGhost(element, ghostElement);
        } catch (e) {
            ghostElement = null; // ghost 失败不阻断逻辑
        }

        isDragging = true;

        // dragstart 派发到被拖元素（页面在此 setData / setDragImage）
        var startEvt = buildDragEvent('dragstart', clientX, clientY);
        element.dispatchEvent(startEvt);

        // dragstart 里若页面取消（preventDefault），按规范不应继续 —— 但我们用
        // 鼠标模拟，无法真正中断手势，这里忽略 cancel 以便拖放库能接收后续事件。
    }

    /* ------------------------------------------------------------------ *
     *  收尾一次交互。
     *  fireDrop=true 表示确实拖过：发 drop（到悬停目标）→ dragend（到被拖元素）。
     *  false 表示只是轻点：仅复位，不发任何拖拽事件，让原生 click 照常触发。
     * ------------------------------------------------------------------ */
    function endInteraction(fireDrop, x, y) {
        // 恢复 draggable，保证下次仍可拖
        if (pendingElement && pendingElement.getAttribute('draggable') !== 'true') {
            pendingElement.setAttribute('draggable', 'true');
        }
        if (draggedElement && draggedElement.getAttribute('draggable') !== 'true') {
            draggedElement.setAttribute('draggable', 'true');
        }

        if (fireDrop && isDragging && draggedElement) {
            // drop 前给悬停目标补最后一次 dragover，确保它“已激活”（某些库要求）。
            if (currentOverElement) {
                try {
                    currentOverElement.dispatchEvent(buildDragEvent('dragover', x, y));
                } catch (e) { /* ignore */ }
                try {
                    currentOverElement.dispatchEvent(buildDragEvent('drop', x, y));
                } catch (e) { /* ignore */ }
            }
            // dragend 派发到被拖元素
            try {
                draggedElement.dispatchEvent(buildDragEvent('dragend', x, y));
            } catch (e) { /* ignore */ }
        }

        clearGhost();

        isDragging = false;
        pendingElement = null;
        draggedElement = null;
        currentOverElement = null;
        sessionDataTransfer = null;
    }

    /* ---------------- pointer cursor（可选） ---------------- */
    if (opts && opts.setPointerCursor) {
        try {
            var head = document.head;
            if (head) {
                var styleTag = document.createElement('style');
                styleTag.textContent = '[draggable="true"] { cursor: pointer; }';
                if (head.children.length > 0) head.insertBefore(styleTag, head.children[0]);
                else head.appendChild(styleTag);
            }
        } catch (e) { /* ignore */ }
    }

    /* ---------------- 事件监听 ---------------- */

    document.addEventListener('mousedown', function (e) {
        if (e.button !== 0) return;               // 仅左键
        if (e.defaultPrevented) return;           // 页面自己处理时别抢
        if (pendingElement || isDragging) endInteraction(false, e.clientX, e.clientY); // 异常态兜底复位

        var draggableElement = findDraggableAncestor(e.target);
        if (!draggableElement) return;

        // 立刻禁用原生 draggable，抑制 WebView2 OLE 把本次手势“拖走”吞掉。
        draggableElement.setAttribute('draggable', 'false');

        // 仅记录候选与起点。是否为拖动由 mousemove 判定位移。
        pendingElement = draggableElement;
        startPosition.x = e.clientX;
        startPosition.y = e.clientY;
    }, true);

    document.addEventListener('mousemove', function (e) {
        // A) 尚未进入拖拽但有候选：判定位移是否超阈值
        if (pendingElement && !isDragging) {
            var dx = e.clientX - startPosition.x;
            var dy = e.clientY - startPosition.y;
            if (Math.abs(dx) >= DRAG_THRESHOLD || Math.abs(dy) >= DRAG_THRESHOLD) {
                var el = pendingElement;
                pendingElement = null;            // 先清，避免 beginDrag 内读到旧态
                beginDrag(el, e.clientX, e.clientY);
            } else {
                return;                           // 位移未超阈值：视为即将点击
            }
        }

        // B) 已在拖动：更新 ghost + 目标悬停(dragenter/dragover/dragleave)
        if (!isDragging || !draggedElement) return;

        if (ghostElement) {
            var gx = e.clientX - startPosition.x;
            var gy = e.clientY - startPosition.y;
            ghostElement.style.transform = 'translate(' + gx + 'px,' + gy + 'px)';
        }
        updateHover(e.clientX, e.clientY);
    }, true);

    document.addEventListener('mouseup', function (e) {
        if (e.button !== 0) return;
        if (!pendingElement && !isDragging) return;

        // 确实拖过 → 收尾发 drop/dragend；仅轻点 → 只复位、让原生 click 走。
        endInteraction(isDragging, e.clientX, e.clientY);
    }, true);

    // 兜底：按住后移出文档 / 窗口失焦时复位，避免 draggable 卡在 false / ghost 残留。
    document.addEventListener('mouseleave', function () {
        if (pendingElement || isDragging) endInteraction(false, 0, 0);
    }, true);
    window.addEventListener('blur', function () {
        if (pendingElement || isDragging) endInteraction(false, 0, 0);
    });
})({ setPointerCursor: true });
