//! Demo 05 - A* on a grid.
//!
//! Click or drag on the grid to paint walls (black). P runs A* from the red
//! circle to the green one and shows its work: every cell the search popped
//! is tinted, the found path is drawn as a line, and the circle glides along
//! it. R puts the circle back; the walls stay.
//!
//! Talking points:
//! - A* is Dijkstra with a pull toward the goal: the heap is ordered by
//!   f = g + h, where g is the cost walked so far and h is a guess of what is
//!   left. With h = 0 you get Dijkstra and the whole grid lights up.
//! - The guess must never overestimate (Manhattan distance is exact on an
//!   empty 4-way grid, so it never does) - that is what makes the first pop of
//!   the goal the shortest path.
//! - The tinted cells are the closed set. Wall the goal in and watch A* flood
//!   everything it can reach before giving up.

use raylib::core::game_loop;
use raylib::prelude::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

const COLS: usize = 20;
const ROWS: usize = 11;
const CELL: f32 = 40.0;
const START: Cell = (0, 0);
const GOAL: Cell = (COLS - 1, ROWS - 1);
const SPEED: f32 = 6.0; // cells per second
const BAR: i32 = 30; // text strip below the grid

/// (column, row)
type Cell = (usize, usize);

fn centre((c, r): Cell) -> Vector2 {
    Vector2::new((c as f32 + 0.5) * CELL, (r as f32 + 0.5) * CELL)
}

fn cell_at(mouse: Vector2) -> Option<Cell> {
    if mouse.x < 0.0 || mouse.y < 0.0 {
        return None;
    }
    let (c, r) = ((mouse.x / CELL) as usize, (mouse.y / CELL) as usize);
    (c < COLS && r < ROWS).then_some((c, r))
}

fn manhattan((ac, ar): Cell, (bc, br): Cell) -> u32 {
    (ac.abs_diff(bc) + ar.abs_diff(br)) as u32
}

/// The four in-bounds neighbours that are not walls.
fn neighbours(walls: &[[bool; COLS]; ROWS], (c, r): Cell) -> impl Iterator<Item = Cell> {
    let mut out = Vec::with_capacity(4);
    if c > 0 {
        out.push((c - 1, r));
    }
    if c + 1 < COLS {
        out.push((c + 1, r));
    }
    if r > 0 {
        out.push((c, r - 1));
    }
    if r + 1 < ROWS {
        out.push((c, r + 1));
    }
    out.into_iter().filter(|&(c, r)| !walls[r][c])
}

/// Returns (path from start to goal, cells popped in the order A* looked at
/// them). The path is empty when the goal is unreachable.
fn astar(walls: &[[bool; COLS]; ROWS], start: Cell, goal: Cell) -> (Vec<Cell>, Vec<Cell>) {
    // Min-heap on (f, g, cell): lowest estimated total first, ties to the
    // one that has walked further (it is closer to done).
    let mut open = BinaryHeap::new();
    let mut g_cost: HashMap<Cell, u32> = HashMap::new();
    let mut came_from: HashMap<Cell, Cell> = HashMap::new();
    let mut closed: HashSet<Cell> = HashSet::new();
    let mut explored = Vec::new();

    g_cost.insert(start, 0);
    open.push(Reverse((manhattan(start, goal), 0u32, start)));

    while let Some(Reverse((_, g, cell))) = open.pop() {
        if !closed.insert(cell) {
            continue; // a stale heap entry - we already expanded this cell
        }
        explored.push(cell);

        if cell == goal {
            let mut path = vec![goal];
            let mut at = goal;
            while let Some(&prev) = came_from.get(&at) {
                path.push(prev);
                at = prev;
            }
            path.reverse();
            return (path, explored);
        }

        for next in neighbours(walls, cell) {
            let tentative = g + 1;
            if tentative < *g_cost.get(&next).unwrap_or(&u32::MAX) {
                g_cost.insert(next, tentative);
                came_from.insert(next, cell);
                open.push(Reverse((tentative + manhattan(next, goal), tentative, next)));
            }
        }
    }
    (Vec::new(), explored)
}

fn main() {
    let (rl, thread) = init()
        .size(COLS as i32 * CELL as i32, ROWS as i32 * CELL as i32 + BAR)
        .title("NTK 05 - A*").build();

    let mut walls = [[false; COLS]; ROWS];
    let mut paint: Option<bool> = None; // what the current drag writes
    let mut explored: Vec<Cell> = Vec::new();
    let mut path: Vec<Cell> = Vec::new();
    let mut progress = 0.0_f32; // how far along `path`, in cells
    let mut moving = false;

    game_loop::run(rl, thread, 60, move |rl, thread| {
        // INPUT
        if rl.is_key_pressed(KeyboardKey::KEY_Q) {
            rl.request_quit();
        }

        let under_mouse = cell_at(rl.get_mouse_position());
        let editable = |cell: Cell| cell != START && cell != GOAL;
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            if let Some(cell) = under_mouse.filter(|&c| editable(c)) {
                paint = Some(!walls[cell.1][cell.0]);
            }
        }
        if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
            paint = None;
        }
        if let (Some(value), Some((c, r))) = (paint, under_mouse.filter(|&c| editable(c))) {
            if walls[r][c] != value {
                walls[r][c] = value;
                // The map changed, so any plan is stale.
                path.clear();
                explored.clear();
                progress = 0.0;
                moving = false;
            }
        }

        if rl.is_key_pressed(KeyboardKey::KEY_P) {
            (path, explored) = astar(&walls, START, GOAL);
            progress = 0.0;
            moving = !path.is_empty();
        }
        if rl.is_key_pressed(KeyboardKey::KEY_R) {
            path.clear();
            explored.clear();
            progress = 0.0;
            moving = false;
        }

        // UPDATE
        if moving {
            let end = (path.len() - 1) as f32;
            progress = (progress + SPEED * rl.get_frame_time()).min(end);
            moving = progress < end;
        }

        // The circle sits between two consecutive path cells.
        let circle = if path.is_empty() {
            centre(START)
        } else {
            let i = (progress.floor() as usize).min(path.len() - 1);
            let j = (i + 1).min(path.len() - 1);
            centre(path[i]).lerp(centre(path[j]), progress - i as f32)
        };

        // DRAW
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::RAYWHITE);

        for &(c, r) in &explored {
            let rect = Rectangle::new(c as f32 * CELL, r as f32 * CELL, CELL, CELL);
            d.draw_rectangle_rec(rect, Color::new(190, 215, 240, 255));
        }
        for r in 0..ROWS {
            for c in 0..COLS {
                let rect = Rectangle::new(c as f32 * CELL, r as f32 * CELL, CELL, CELL);
                if walls[r][c] {
                    d.draw_rectangle_rec(rect, Color::BLACK);
                }
                d.draw_rectangle_lines_ex(rect, 1.0, Color::LIGHTGRAY);
            }
        }
        if path.len() > 1 {
            let points: Vec<Vector2> = path.iter().map(|&cell| centre(cell)).collect();
            for pair in points.windows(2) {
                d.draw_line_ex(pair[0], pair[1], 4.0, Color::new(60, 110, 200, 255));
            }
        }
        d.draw_circle_v(centre(GOAL), 14.0, Color::LIME);
        d.draw_circle_v(circle, 14.0, Color::RED);

        // let bar_y = ROWS as i32 * CELL as i32 + 8;
        // d.draw_text("Click/drag: walls   P: find path   R: reset   Q: quit", 12, bar_y, 16, Color::DARKGRAY);
        // let info = format!("explored {} cells, path {} cells", explored.len(), path.len());
        // let width = d.measure_text(&info, 16);
        // d.draw_text(&info, COLS as i32 * CELL as i32 - width - 12, bar_y, 16, Color::DARKGRAY);
    });
}
