/**
 * Polyfill for WebView2 Drag and Drop Bug
 * 
 * This polyfill patches a known bug in WebView2 where drag and drop functionalities are 
 * not working as expected. This script provides a mock implementation to temporarily 
 * overcome the issue.
 *
 * More information:
 * - https://github.com/MicrosoftEdge/WebView2Feedback/issues/2805
 * - https://github.com/dotnet/maui/issues/2205
 *
 *
 * Developers integrating this polyfill should implement their own feature detection or 
 * settings toggle to decide when to apply this fix.
 *
 * Copyright (c) 2023 Iain Fraser
 * 
 * Permission is hereby granted, free of charge, to any person obtaining a copy of 
 * this software and associated documentation files (the "Software"), to deal in 
 * the Software without restriction, including without limitation the rights to use,
 * copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the 
 * Software, and to permit persons to whom the Software is furnished to do so, 
 * subject to the following conditions:
 * 
 * The above copyright notice and this permission notice shall be included in all 
 * copies or substantial portions of the Software.
 * 
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, 
 * INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A 
 * PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT 
 * HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION 
 * OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE 
 * SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */
(function (opts) {
    // 拖动判定阈值（像素）：按下左键后，指针位移超过该值才判定为“拖动”。
    // 若只是轻点（位移不超过阈值），不进入拖拽流程，也不创建 ghost 遮挡层，
    // 从而保证 draggable 元素上的原生 click 仍能正常触发（可拖可点）。
    const DRAG_THRESHOLD = (opts && opts.dragThreshold) || 4;

    let isDragging = false;      // 已判定为拖动（位移超过阈值、ghost 已创建）
    let pendingElement = null;   // 刚按下、尚未判定拖/点的 draggable 候选元素
    let draggedElement = null;   // 真正被拖动的元素
    let startPosition = { x: 0, y: 0 };
    let currentOverElement = null;
    let ghostElement = null;
    // 整个拖拽会话共享的 DataTransfer：dragstart 里 setData 的数据，必须能在随后的
    // dragover / drop / dragend 里用 getData 取回（真实浏览器如此）。若每次事件都 new
    // 一个空的，drop 处理器会读不到 dragstart 写入的数据，导致“能拖、能点，但放不到
    // 具体位置”。
    let sessionDataTransfer = null;

    /**
     * Searches for the nearest ancestor of the provided element that has the draggable attribute set to true.
     * @param {HTMLElement} element - The starting element to begin the search from.
     * @returns {HTMLElement|null} - The draggable ancestor if found, otherwise null.
     */
    function findDraggableAncestor(element) {
        while (element && element !== document.body) {
            if (element.getAttribute('draggable') === 'true') {
                return element;
            }
            element = element.parentElement;
        }
        return null;
    }

    /**
     * Mock DataTransfer class mimicking the DataTransfer object available in drag & drop operations.
     */
    function MockDataTransfer() {
        this.dropEffect = 'move';
        this.effectAllowed = 'all';
        this.files = [];
        this.items = [];
        this.types = [];
        this.setData = function (format, data) {
            this.items.push({ format, data });
            this.types.push(format);
        };
        this.getData = function (format) {
            let item = this.items.find(i => i.format === format);
            return item ? item.data : '';
        };
        this.clearData = function (format) {
            this.items = this.items.filter(i => i.format !== format);
            this.types = this.types.filter(t => t !== format);
        };
    }

    /**
     * Creates a mock drag event.
     * @param {string} type - The type of the drag event (e.g., "dragstart", "dragend", etc.)
     * @param {Object} options - Options to be passed to the event.
     * @param {Object} [dataTransfer] - 复用的 DataTransfer。一次拖拽会话中所有事件必须共享
     *   同一个，否则 dragstart 里 setData 的数据在 drop 时取不回。
     * @returns {Event} - A mocked drag event.
     */
    function createMockDragEvent(type, options, dataTransfer) {
        let event = new Event(type, options);
        event.dataTransfer = dataTransfer || new MockDataTransfer();
        return event;
    }

    /**
     * Applies computed styles from the original element to the ghost element.
     * @param {HTMLElement} original - The original draggable element.
     * @param {HTMLElement} ghost - The cloned "ghost" element.
     */
    function applyStylesToGhost(original, ghost) {
        let computedStyles = window.getComputedStyle(original);
        for (let prop of computedStyles) {
            ghost.style[prop] = computedStyles[prop];
        }

        // Positioning and z-index
        let rect = original.getBoundingClientRect();
        ghost.style.position = 'fixed';
        ghost.style.left = `${rect.left}px`;
        ghost.style.top = `${rect.top}px`;
        ghost.style.zIndex = '1000';
        ghost.style.opacity = '0.7';

        document.body.appendChild(ghost);
    }

    /**
     * 移除并清空 ghost（若存在）。无论是否发生过拖动都必须调用，防止遮挡层残留。
     */
    function clearGhost() {
        if (ghostElement && ghostElement.parentNode) {
            ghostElement.parentNode.removeChild(ghostElement);
        }
        ghostElement = null;
    }

    /**
     * 真正开始一次拖动：创建 ghost 并派发 dragstart。
     * 仅在 mousemove 位移超过阈值后被调用一次。
     */
    function beginDrag(element, clientX, clientY) {
        draggedElement = element;

        // Create a "ghost" clone for visual dragging
        ghostElement = element.cloneNode(true);
        applyStylesToGhost(element, ghostElement);

        isDragging = true;

        // 会话级共享 DataTransfer：后续 dragover/drop/dragend 都要能读到 dragstart 里
        // setData 的数据，因此复用同一实例。
        sessionDataTransfer = new MockDataTransfer();

        // Fire dragstart event
        element.dispatchEvent(createMockDragEvent('dragstart', {
            bubbles: true,
            cancelable: true,
            clientX: clientX,
            clientY: clientY
        }, sessionDataTransfer));
    }

    /**
     * 复位一次鼠标交互的完整状态。
     * fireDrop 仅在“确实发生过拖动”时为 true，此时派发 drop/dragend；
     * 若只是轻点（从未超阈值）则 fireDrop 为 false，不派发任何拖拽事件，
     * 让浏览器的原生 click 照常触发。
     */
    function endInteraction(fireDrop) {
        // 恢复 draggable，保证元素下次仍可拖动
        if (pendingElement && pendingElement.getAttribute('draggable') !== 'true') {
            pendingElement.setAttribute('draggable', 'true');
        }
        if (draggedElement && draggedElement.getAttribute('draggable') !== 'true') {
            draggedElement.setAttribute('draggable', 'true');
        }

        if (fireDrop && isDragging && draggedElement) {
            // Fire drop event if we have a target
            if (currentOverElement) {
                currentOverElement.dispatchEvent(createMockDragEvent('drop', {
                    bubbles: true,
                    cancelable: true
                }, sessionDataTransfer));
            }

            // Fire dragend event
            draggedElement.dispatchEvent(createMockDragEvent('dragend', {
                bubbles: true,
                cancelable: true
            }, sessionDataTransfer));
        }

        // 无论是否拖动，都清理 ghost（若存在），避免遮挡层残留导致后续点击被吞
        clearGhost();

        isDragging = false;
        pendingElement = null;
        draggedElement = null;
        currentOverElement = null;
        sessionDataTransfer = null;
    }

    if (opts.setPointerCursor) {
        // 该 polyfill 常以“初始化脚本”形式在 HTML 解析前注入，此时 document.head 可能为 null，
        // 故先判空再插入样式，避免初始化阶段抛错导致后续 mousedown/move/up 监听器未能注册。
        const head = document.head;
        if (head) {
            const styleTag = document.createElement('style');
            styleTag.textContent = '[draggable="true"] { cursor: pointer; }';

            // Insert the style tag as the first child of the head element
            if (head.children.length > 0) {
                head.insertBefore(styleTag, head.children[0]);
            } else {
                head.appendChild(styleTag);
            }
        }
    }

    document.addEventListener('mousedown', function (e) {
        // Only proceed if the left mouse button is clicked
        if (e.button !== 0) return;

        // 若上一次交互尚未收尾（如 mouseup 落在窗口外），先复位，避免状态错乱
        if (pendingElement || isDragging) {
            endInteraction(false);
        }

        let draggableElement = findDraggableAncestor(e.target);

        if (!draggableElement) return;

        // 立刻禁用原生 draggable，抑制 WebView2 的 OLE 拖放把本次操作“拖走”吞掉
        //（这正是该 polyfill 要绕过的原生 bug）。mouseup 时按需恢复。
        draggableElement.setAttribute('draggable', 'false');

        // 仅记录候选与起点。先不创建 ghost、不进入拖拽——是否为拖动交给 mousemove 判定位移。
        pendingElement = draggableElement;
        startPosition.x = e.clientX;
        startPosition.y = e.clientY;
    });

    document.addEventListener('mousemove', function (e) {
        // A) 尚未进入拖拽但有候选元素：判定位移是否超阈值
        if (pendingElement && !isDragging) {
            let dx = e.clientX - startPosition.x;
            let dy = e.clientY - startPosition.y;
            if (Math.abs(dx) >= DRAG_THRESHOLD || Math.abs(dy) >= DRAG_THRESHOLD) {
                // 位移超过阈值 → 判定为拖动，真正开始
                beginDrag(pendingElement, e.clientX, e.clientY);
                pendingElement = null;
            }
            // 位移未超阈值：仍视为“即将点击”，不做任何事，保证原生 click 正常。
            return;
        }

        // B) 已在拖动：更新 ghost 位置 + 合成 dragover
        if (!isDragging || !ghostElement) return;

        let x = e.clientX - startPosition.x;
        let y = e.clientY - startPosition.y;
        ghostElement.style.transform = `translate(${x}px, ${y}px)`;

        // Check for drag over target
        ghostElement.style.display = 'none';
        let elementBelow = document.elementFromPoint(e.clientX, e.clientY);
        ghostElement.style.display = '';

        if (elementBelow && elementBelow !== currentOverElement) {
            elementBelow.dispatchEvent(createMockDragEvent('dragover', {
                bubbles: true,
                cancelable: true,
                clientX: e.clientX,
                clientY: e.clientY
            }, sessionDataTransfer));
            currentOverElement = elementBelow;
        }
    });

    document.addEventListener('mouseup', function (e) {
        // Only drop and dragend if dragging the event was triggerd with button 0 (left-click)
        if (e.button !== 0) return;

        if (!pendingElement && !isDragging) return;

        // fireDrop 仅在“确实发生过拖动”时为 true：
        //   · 拖动过 → 派发 drop/dragend + 清 ghost；
        //   · 仅轻点 → 不派发拖拽事件、不创建过 ghost，原生 click 由浏览器照常触发。
        endInteraction(isDragging);
    });

    // 兜底：若指针按住后移出窗口、mouseup 未触发（松在窗口外），
    // 则在此复位，避免 draggable 长时间停留在 false / ghost 残留。
    document.addEventListener('mouseleave', function () {
        if (pendingElement || isDragging) {
            endInteraction(false);
        }
    });

    // 再兜底：窗口失焦（如拖到别的程序）时同样复位。
    window.addEventListener('blur', function () {
        if (pendingElement || isDragging) {
            endInteraction(false);
        }
    });

})({ setPointerCursor: true });
