//! Walkability + A* pathfinding on the LMF cell grid (16px cells).
//!
//! Cell attribute word (LMF H+0x08, LZO, u32 per cell), as tested by the original
//! collision code (`cell & 0x60000000`) and verified visually on maps 1/2/30:
//! - bit 30 (0x4000_0000): blocked (walls, furniture feet, map border)
//! - bit 29 (0x2000_0000): also tested by the original collision mask; not set on sampled maps
//! - bits 0..7: region index, 1-based into the map's region list (exits / triggers, `Region.v[1]` = talk script)
//! - bits 12..15: unknown nibble (values 1, d, e, f on floors/stairs; height or path layer?)
use crate::lmf::{LmfMap, TILE};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub const BLOCK_MASK: u32 = 0x6000_0000;
pub const REGION_MASK: u32 = 0x0000_00FF;

#[derive(Clone, Debug)]
pub struct NavGrid {
    pub w: i32,
    pub h: i32,
    pub cells: Vec<u32>,
}

#[derive(Copy, Clone, PartialEq)]
struct Node(f32, usize);
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering { o.0.total_cmp(&self.0) }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) }
}

const DIRS: [(i32, i32, f32); 8] = [
    (1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0),
    (1, 1, std::f32::consts::SQRT_2), (1, -1, std::f32::consts::SQRT_2), (-1, 1, std::f32::consts::SQRT_2), (-1, -1, std::f32::consts::SQRT_2),
];

impl NavGrid {
    pub fn new(w: i32, h: i32, cells: Vec<u32>) -> Self {
        assert_eq!(cells.len(), (w * h) as usize);
        Self { w, h, cells }
    }
    pub fn from_map(m: &LmfMap) -> Self {
        let mut cells = m.cells.clone();
        cells.resize(m.w as usize * m.h as usize, BLOCK_MASK);
        Self::new(m.w as i32, m.h as i32, cells)
    }
    pub fn cell_of(&self, x: f32, y: f32) -> (i32, i32) {
        ((x / TILE as f32).floor() as i32, (y / TILE as f32).floor() as i32)
    }
    pub fn center(cx: i32, cy: i32) -> (f32, f32) {
        ((cx as f32 + 0.5) * TILE as f32, (cy as f32 + 0.5) * TILE as f32)
    }
    pub fn get(&self, cx: i32, cy: i32) -> Option<u32> {
        (cx >= 0 && cy >= 0 && cx < self.w && cy < self.h).then(|| self.cells[(cy * self.w + cx) as usize])
    }
    pub fn walkable(&self, cx: i32, cy: i32) -> bool {
        self.get(cx, cy).is_some_and(|c| c & BLOCK_MASK == 0)
    }
    pub fn walkable_px(&self, x: f32, y: f32) -> bool {
        let (cx, cy) = self.cell_of(x, y);
        self.walkable(cx, cy)
    }
    /// Region index (1-based) under a pixel, 0 = none.
    pub fn region_px(&self, x: f32, y: f32) -> u32 {
        let (cx, cy) = self.cell_of(x, y);
        self.get(cx, cy).map_or(0, |c| c & REGION_MASK)
    }
    /// Nearest walkable cell (BFS rings) to a cell.
    pub fn nearest_walkable(&self, cx: i32, cy: i32) -> Option<(i32, i32)> {
        if self.walkable(cx, cy) {
            return Some((cx, cy));
        }
        for r in 1..self.w.max(self.h) {
            let mut best: Option<((i32, i32), i32)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r { continue; }
                    let (x, y) = (cx + dx, cy + dy);
                    if self.walkable(x, y) {
                        let d = dx * dx + dy * dy;
                        if best.is_none_or(|b| d < b.1) { best = Some(((x, y), d)); }
                    }
                }
            }
            if let Some((c, _)) = best { return Some(c); }
        }
        None
    }
    fn step_ok(&self, x: i32, y: i32, dx: i32, dy: i32) -> bool {
        self.walkable(x + dx, y + dy) && (dx == 0 || dy == 0 || (self.walkable(x + dx, y) && self.walkable(x, y + dy)))
    }
    /// 8-direction A* (no corner cutting) between cells. Returns the cell path including both ends.
    pub fn astar(&self, from: (i32, i32), to: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        if !self.walkable(from.0, from.1) || !self.walkable(to.0, to.1) { return None; }
        let n = (self.w * self.h) as usize;
        let idx = |x: i32, y: i32| (y * self.w + x) as usize;
        let hfn = |x: i32, y: i32| {
            let (dx, dy) = ((x - to.0).abs() as f32, (y - to.1).abs() as f32);
            dx.max(dy) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dy)
        };
        let mut g = vec![f32::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        let s = idx(from.0, from.1);
        g[s] = 0.0;
        open.push(Node(hfn(from.0, from.1), s));
        while let Some(Node(_, cur)) = open.pop() {
            let (x, y) = ((cur as i32) % self.w, (cur as i32) / self.w);
            if (x, y) == to {
                let mut p = vec![(x, y)];
                let mut c = cur;
                while prev[c] != usize::MAX { c = prev[c]; p.push(((c as i32) % self.w, (c as i32) / self.w)); }
                p.reverse();
                return Some(p);
            }
            for &(dx, dy, cost) in &DIRS {
                if !self.step_ok(x, y, dx, dy) { continue; }
                let ni = idx(x + dx, y + dy);
                let ng = g[cur] + cost;
                if ng < g[ni] {
                    g[ni] = ng;
                    prev[ni] = cur;
                    open.push(Node(ng + hfn(x + dx, y + dy), ni));
                }
            }
        }
        None
    }
    /// Straight segment test in pixel space (samples every 4 px).
    pub fn line_clear(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let steps = ((dx * dx + dy * dy).sqrt() / 4.0).ceil().max(1.0) as i32;
        (0..=steps).all(|i| {
            let t = i as f32 / steps as f32;
            self.walkable_px(a.0 + dx * t, a.1 + dy * t)
        })
    }
    /// Pixel waypoints from `from` to `to` (goal snapped to the nearest walkable cell), smoothed by line of sight.
    pub fn find_path(&self, from: (f32, f32), to: (f32, f32)) -> Option<Vec<(f32, f32)>> {
        let (sx, sy) = self.cell_of(from.0, from.1);
        let s = self.nearest_walkable(sx, sy)?;
        let (gx, gy) = self.cell_of(to.0, to.1);
        let g = self.nearest_walkable(gx, gy)?;
        let cells = self.astar(s, g)?;
        let goal = if (gx, gy) == g { to } else { Self::center(g.0, g.1) };
        let mut pts: Vec<(f32, f32)> = cells.iter().skip(1).map(|&(x, y)| Self::center(x, y)).collect();
        if let Some(l) = pts.last_mut() { *l = goal } else { pts.push(goal) }
        let mut out = Vec::new();
        let mut cur = from;
        let mut i = 0;
        while i < pts.len() {
            let mut j = pts.len() - 1;
            while j > i && !self.line_clear(cur, pts[j]) { j -= 1; }
            out.push(pts[j]);
            cur = pts[j];
            i = j + 1;
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grid(rows: &[&str]) -> NavGrid {
        let h = rows.len() as i32;
        let w = rows[0].len() as i32;
        let cells = rows.iter().flat_map(|r| r.bytes().map(|b| match b { b'#' => 0x4000_0000, b'0'..=b'9' => (b - b'0') as u32, _ => 0 })).collect();
        NavGrid::new(w, h, cells)
    }
    #[test]
    fn astar_goes_around_wall_without_corner_cut() {
        let g = grid(&["....", ".##.", ".#..", "...."]);
        let p = g.astar((0, 0), (2, 2)).unwrap();
        assert_eq!(p.first(), Some(&(0, 0)));
        assert_eq!(p.last(), Some(&(2, 2)));
        for w in p.windows(2) {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            assert!(g.step_ok(w[0].0, w[0].1, dx, dy));
        }
        assert!(p.iter().all(|&(x, y)| g.walkable(x, y)));
    }
    #[test]
    fn unreachable_and_snap() {
        let g = grid(&["..#.", "..#.", "..#."]);
        assert!(g.astar((0, 0), (3, 0)).is_none());
        assert_eq!(g.nearest_walkable(2, 1), Some((1, 1)));
        let p = g.find_path((8.0, 8.0), (40.0, 40.0)).unwrap(); // click on the wall -> nearest floor
        let last = *p.last().unwrap();
        assert!(g.walkable_px(last.0, last.1));
    }
    #[test]
    fn region_bits() {
        let g = grid(&[".3", "#."]);
        assert_eq!(g.region_px(20.0, 2.0), 3);
        assert!(!g.walkable_px(1.0, 17.0));
    }
}