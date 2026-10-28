//! 悬浮球单元测试 —— v1.81 起覆盖「四边吸附几何」纯函数（窗口 == 球，无活柱）。
//! 旧的 live_px / live_offset 用例随活柱退役一并删除。

use super::*;

#[test]
fn decide_edge_detects_all_four_edges() {
    // 显示器 1920×1080，悬浮球 56×56。
    let mon = (0, 0, 1920, 1080);
    let snap = 42;
    // 贴左缘（gap=0）→ Left
    assert_eq!(decide_edge((0, 100, 56, 56), mon, snap), DockEdge::Left);
    // 左缘外一点（gap=-10，仍在容差内）→ Left
    assert_eq!(decide_edge((-10, 100, 56, 56), mon, snap), DockEdge::Left);
    // 贴右缘：x = 1920-56 = 1864
    assert_eq!(decide_edge((1864, 100, 56, 56), mon, snap), DockEdge::Right);
    // 贴上缘：y = 0
    assert_eq!(decide_edge((500, 0, 56, 56), mon, snap), DockEdge::Top);
    // 贴下缘：y = 1080-56 = 1024
    assert_eq!(
        decide_edge((500, 1024, 56, 56), mon, snap),
        DockEdge::Bottom
    );
    // 离四边都远 → None
    assert_eq!(decide_edge((500, 500, 56, 56), mon, snap), DockEdge::None);
    // 超出容差 → None
    assert_eq!(decide_edge((100, 100, 56, 56), mon, snap), DockEdge::None);
}

#[test]
fn decide_edge_requires_overlap_on_the_other_axis() {
    let mon = (0, 0, 1920, 1080);
    // 球完全在屏幕下方：即使 x 贴左缘也不吸附（纵向无重叠）。
    assert_eq!(decide_edge((0, 2000, 56, 56), mon, 42), DockEdge::None);
    // 球完全在屏幕右侧：即使 y 贴上缘也不吸附（横向无重叠）。
    assert_eq!(decide_edge((3000, 0, 56, 56), mon, 42), DockEdge::None);
}

#[test]
fn decide_edge_picks_nearest_edge_and_breaks_corner_ties() {
    let mon = (0, 0, 1920, 1080);
    // 屏幕左上角：left gap=0、top gap=0 —— 平局按声明顺序 left → right → top → bottom 取先。
    assert_eq!(decide_edge((0, 0, 56, 56), mon, 42), DockEdge::Left);
    // 左 gap=20、上 gap=10：上更近 → Top。
    assert_eq!(decide_edge((20, 10, 56, 56), mon, 42), DockEdge::Top);
    // 右下角平局：right 与 bottom 都是 gap=0，right 在声明顺序中先 → Right。
    assert_eq!(
        decide_edge((1920 - 56, 1080 - 56, 56, 56), mon, 42),
        DockEdge::Right
    );
    // 右 gap=30、下 gap=5：下更近 → Bottom。
    assert_eq!(
        decide_edge((1920 - 56 - 30, 1080 - 56 - 5, 56, 56), mon, 42),
        DockEdge::Bottom
    );
}

#[test]
fn collapsed_and_expanded_x_are_inverses() {
    let (mx, mw, ww, peek) = (0, 1920, 56, 28);
    // 左吸附：展开 x=0，半隐藏 x = 0-(56-28) = -28（只露右侧 28px = 半球）。
    assert_eq!(expanded_x(DockEdge::Left, mx, mw, ww), 0);
    assert_eq!(collapsed_x(DockEdge::Left, mx, mw, ww, peek), -28);
    // 右吸附：展开 x=1864，半隐藏 x = 1920-28 = 1892（只露左侧 28px）。
    assert_eq!(expanded_x(DockEdge::Right, mx, mw, ww), 1864);
    assert_eq!(collapsed_x(DockEdge::Right, mx, mw, ww, peek), 1892);
}

#[test]
fn collapsed_and_expanded_y_mirror_the_x_axis() {
    let (my, mh, wh, peek) = (0, 1080, 56, 28);
    // 上吸附：展开 y=0，半隐藏 y=-28（只露下半球）。
    assert_eq!(expanded_y(DockEdge::Top, my, mh, wh), 0);
    assert_eq!(collapsed_y(DockEdge::Top, my, mh, wh, peek), -28);
    // 下吸附：展开 y=1024，半隐藏 y=1080-28=1052。
    assert_eq!(expanded_y(DockEdge::Bottom, my, mh, wh), 1024);
    assert_eq!(collapsed_y(DockEdge::Bottom, my, mh, wh, peek), 1052);
}

#[test]
fn clamp_y_keeps_window_inside_monitor() {
    // 太高（y<上沿）→ 贴顶；太低 → 贴底（my+mh-wh）。
    assert_eq!(clamp_y(-50, 56, 0, 1080), 0);
    assert_eq!(clamp_y(2000, 56, 0, 1080), 1024);
    assert_eq!(clamp_y(200, 56, 0, 1080), 200);
}

#[test]
fn clamp_x_mirrors_clamp_y() {
    assert_eq!(clamp_x(-50, 56, 0, 1920), 0);
    assert_eq!(clamp_x(3000, 56, 0, 1920), 1864);
    assert_eq!(clamp_x(500, 56, 0, 1920), 500);
    // 负原点显示器（多显示器）同样成立。
    assert_eq!(clamp_x(-3000, 56, -1920, 1920), -1920);
}

#[test]
fn peek_scales_with_dpi_but_never_below_floor() {
    assert_eq!(peek_px(1.0), 28);
    assert_eq!(peek_px(1.5), 42);
    // 异常的极小缩放也至少留 10px，保证半球可悬停。
    assert_eq!(peek_px(0.1), 10);
}

#[test]
fn min_visible_scales_with_floor() {
    assert_eq!(min_visible_px(1.0), 16);
    assert_eq!(min_visible_px(1.5), 24);
    assert_eq!(min_visible_px(0.1), 8);
}

#[test]
fn ball_px_is_a_scaled_square_with_floor() {
    assert_eq!(ball_px(1.0), (56, 56));
    assert_eq!(ball_px(2.0), (112, 112));
    assert_eq!(ball_px(1.5), (84, 84));
    // 异常小缩放下也至少 32 物理像素，避免窗口退化。
    assert_eq!(ball_px(0.1), (32, 32));
}

#[test]
fn dock_target_pos_moves_only_the_docked_axis() {
    // 显示器 1920×1080、球 56×56（100% 缩放）、peek=28。cur 是当前物理矩形。
    let mon = (0, 0, 1920, 1080);
    let cur = (500, 500, 56, 56);
    // 左吸附：x 走 expanded/collapsed，y 保持（钳制后）500。
    let d = Dock {
        edge: DockEdge::Left,
        mx: mon.0,
        my: mon.1,
        mw: mon.2,
        mh: mon.3,
        collapsed: false,
    };
    assert_eq!(dock_target_pos(d, cur, 1.0), (0, 500));
    assert_eq!(
        dock_target_pos(
            Dock {
                collapsed: true,
                ..d
            },
            cur,
            1.0
        ),
        (-28, 500)
    );
    // 上吸附：y 走 expanded/collapsed，x 保持 500。
    let dt = Dock {
        edge: DockEdge::Top,
        ..d
    };
    assert_eq!(dock_target_pos(dt, cur, 1.0), (500, 0));
    assert_eq!(
        dock_target_pos(
            Dock {
                collapsed: true,
                ..dt
            },
            cur,
            1.0
        ),
        (500, -28)
    );
    // 自由轴越界时钳回显示器内（下吸附 + cur.x 超右缘 → x 钳到 1864）。
    let db = Dock {
        edge: DockEdge::Bottom,
        ..d
    };
    assert_eq!(dock_target_pos(db, (3000, 500, 56, 56), 1.0), (1864, 1024));
}

#[test]
fn point_in_rect_uses_half_open_bounds() {
    // 矩形 [10, 66) × [20, 76)
    assert!(point_in_rect(10, 20, 10, 20, 56, 56));
    assert!(point_in_rect(65, 75, 10, 20, 56, 56));
    // 右 / 下沿是开区间：恰好落在外边上不算在内。
    assert!(!point_in_rect(66, 50, 10, 20, 56, 56));
    assert!(!point_in_rect(50, 76, 10, 20, 56, 56));
    assert!(!point_in_rect(9, 50, 10, 20, 56, 56));
}

#[test]
fn clamp_visible_keeps_a_strip_but_allows_crossing() {
    // 显示器 x ∈ [0, 1920)，球宽 84（150% DPI），最小可见 24。
    let (ww, minv, lo, hi) = (84, 24, 0, 1920);
    // 正常位置原样。
    assert_eq!(clamp_visible(500, ww, lo, hi, minv), 500);
    // 甩向最右：最多到 1920-24=1896，留 24px 在屏内。
    assert_eq!(clamp_visible(3000, ww, lo, hi, minv), 1896);
    // 甩向最左：最多到 0-84+24=-60，仍留 24px 在屏内（其余可越过左缘跨屏）。
    assert_eq!(clamp_visible(-500, ww, lo, hi, minv), -60);
    // 负原点显示器（多显示器）同样成立。
    assert_eq!(clamp_visible(-2000, ww, -1920, 0, minv), -1980);
}

#[test]
fn free_position_is_clamped_fully_inside_monitor_at_drag_end() {
    // 自由态松手时的钳制规则（与 on_drag_end 内联同式）：
    // x ∈ [mx, mx+mw-ww]，y 走 clamp_y。显示器 1920×1080、球 84 物理像素。
    let (mx, my, mw, mh, ww, wh) = (0, 0, 1920, 1080, 84, 84);
    assert_eq!(clamp_x(500, ww, mx, mw), 500);
    assert_eq!(clamp_x(2000, ww, mx, mw), 1920 - 84);
    assert_eq!(clamp_x(-120, ww, mx, mw), 0);
    assert_eq!(clamp_y(2000, wh, my, mh), 1080 - 84);
}

#[test]
fn dock_edge_serde_round_trips_all_variants() {
    // 落盘契约：小写字符串；旧数据缺省 none。top / bottom 是 v1.81 四边吸附新增。
    for (edge, s) in [
        (DockEdge::None, "\"none\""),
        (DockEdge::Left, "\"left\""),
        (DockEdge::Right, "\"right\""),
        (DockEdge::Top, "\"top\""),
        (DockEdge::Bottom, "\"bottom\""),
    ] {
        assert_eq!(serde_json::to_string(&edge).unwrap(), s);
        let back: DockEdge = serde_json::from_str(s).unwrap();
        assert_eq!(edge_str(back), edge_str(edge));
    }
    // 旧版本落盘文件没有 edge 字段 / 字段为空：按 none 解析，不得报错。
    let g: crate::store::FloatWinGeom =
        serde_json::from_str(r#"{"x":1,"y":2,"width":56,"height":56}"#).unwrap();
    assert_eq!(edge_str(g.edge), "none");
}
