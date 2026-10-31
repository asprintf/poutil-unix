/* SPDX-License-Identifier: GPL-2.0-only */
/* tsort - topological sort. */

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io::{self, Write};

const SYNOPSIS: &str = "tsort [file]";

/*
 * The partial order read so far. Items get a number in order of first
 * appearance; "a b" makes an edge a -> b ("a comes before b"), "a a" only
 * makes sure a exists. Memory is proportional to the items and pairs, never
 * to the text: the input is tokenized block by block.
 */
#[derive(Default)]
struct Graph {
    ids: HashMap<Vec<u8>, usize>,
    succ: Vec<Vec<usize>>,
    indeg: Vec<usize>,
    /* the first item of a pair whose second item has not come yet */
    open: Option<usize>,
    /* the item being read, when a block ended inside it */
    cur: Vec<u8>,
    in_token: bool,
}

/* The bytes isspace() accepts in the C locale. */
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

impl Graph {
    fn intern(&mut self, tok: &[u8]) -> usize {
        if let Some(&i) = self.ids.get(tok) {
            return i;
        }
        let i = self.succ.len();
        self.ids.insert(tok.to_vec(), i);
        self.succ.push(Vec::new());
        self.indeg.push(0);
        i
    }

    fn token(&mut self, tok: &[u8]) {
        let id = self.intern(tok);
        match self.open.take() {
            None => self.open = Some(id),
            Some(a) => {
                if a != id {
                    self.succ[a].push(id);
                    self.indeg[id] += 1;
                }
            }
        }
    }

    /* Feed the next block of input. */
    fn feed(&mut self, block: &[u8]) {
        let mut i = 0;
        while i < block.len() {
            if is_space(block[i]) {
                if self.in_token {
                    let t = std::mem::take(&mut self.cur);
                    self.in_token = false;
                    self.token(&t);
                }
                i += 1;
                continue;
            }
            let mut j = i;
            while j < block.len() && !is_space(block[j]) {
                j += 1;
            }
            if j == block.len() {
                /* the token may go on in the next block */
                self.cur.extend_from_slice(&block[i..j]);
                self.in_token = true;
            } else if self.in_token {
                self.cur.extend_from_slice(&block[i..j]);
                let t = std::mem::take(&mut self.cur);
                self.in_token = false;
                self.token(&t);
            } else {
                self.token(&block[i..j]);
            }
            i = j;
        }
    }

    /* The end of the input. False if the last item has no partner (odd data). */
    fn finish(&mut self) -> bool {
        if self.in_token {
            let t = std::mem::take(&mut self.cur);
            self.in_token = false;
            self.token(&t);
        }
        self.open.is_none()
    }

    /* The items in the order they were first seen. */
    fn names(&mut self) -> Vec<Vec<u8>> {
        let mut names = vec![Vec::new(); self.succ.len()];
        for (k, v) in std::mem::take(&mut self.ids) {
            names[v] = k;
        }
        names
    }
}

/*
 * Order the items: every item comes after the items that precede it. Among
 * the items that may come next the one seen first is taken, so the output
 * follows the input as far as the constraints allow (the spec's example
 * "a b c c d e g g f g e f h h" gives a b c d e f g h).
 *
 * If the pairs contain a cycle, one edge of it is dropped, the cycle is
 * reported (as the items in order along it) and the sort goes on. Returns
 * the order and the cycles. Time is O((items + pairs) log items) plus the
 * cost of finding each cycle.
 */
fn sort(g: &mut Graph) -> (Vec<usize>, Vec<Vec<usize>>) {
    let n = g.succ.len();
    let mut ready: BinaryHeap<Reverse<usize>> = (0..n).filter(|&i| g.indeg[i] == 0).map(Reverse).collect();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut cycles = Vec::new();
    /* predecessor lists, built when the first cycle shows up */
    let mut pred: Option<Vec<Vec<usize>>> = None;
    let mut lowest = 0;
    while order.len() < n {
        while let Some(Reverse(u)) = ready.pop() {
            done[u] = true;
            order.push(u);
            for k in 0..g.succ[u].len() {
                let v = g.succ[u][k];
                g.indeg[v] -= 1;
                if g.indeg[v] == 0 {
                    ready.push(Reverse(v));
                }
            }
        }
        if order.len() == n {
            break;
        }
        /*
         * Everything left has a predecessor that is left too, so walking
         * backwards from any of it ends in a cycle.
         */
        let p = pred.get_or_insert_with(|| {
            let mut p = vec![Vec::new(); n];
            for u in 0..n {
                for &v in &g.succ[u] {
                    p[v].push(u);
                }
            }
            p
        });
        while done[lowest] {
            lowest += 1;
        }
        let mut path = vec![lowest];
        let mut at: HashMap<usize, usize> = HashMap::new();
        at.insert(lowest, 0);
        let cycle = loop {
            let cur = *path.last().unwrap_or(&lowest);
            let before = *p[cur].iter().find(|&&x| !done[x]).unwrap_or(&cur);
            if let Some(&i) = at.get(&before) {
                /* path[i..] runs against the edges: reverse it */
                let mut c: Vec<usize> = path[i..].to_vec();
                c.reverse();
                break c;
            }
            at.insert(before, path.len());
            path.push(before);
        };
        /*
         * Drop the edge into the item of the cycle seen first, so that the
         * output stays as close to the input order as possible.
         */
        let k = (0..cycle.len()).min_by_key(|&i| cycle[i]).unwrap_or(0);
        let to = cycle[k];
        let from = cycle[(k + cycle.len() - 1) % cycle.len()];
        if let Some(i) = g.succ[from].iter().position(|&x| x == to) {
            g.succ[from].swap_remove(i);
        }
        if let Some(i) = p[to].iter().position(|&x| x == from) {
            p[to].swap_remove(i);
        }
        g.indeg[to] -= 1;
        if g.indeg[to] == 0 {
            ready.push(Reverse(to));
        }
        cycles.push(cycle);
    }
    (order, cycles)
}

fn warn_item(prog: &[u8], name: &[u8]) {
    let mut e = io::stderr().lock();
    let _ = e.write_all(prog);
    let _ = e.write_all(b": ");
    let _ = e.write_all(name);
    let _ = e.write_all(b"\n");
}

fn main() {
    posix::init();
    let args = posix::args();
    let prog = posix::progname(&args);
    let files = posix::no_options(&args, SYNOPSIS);
    if files.len() > 1 {
        posix::usage(SYNOPSIS);
    }
    let name: &[u8] = files.first().map(|v| v.as_slice()).unwrap_or(b"-");
    let mut h = match posix::open_input(name) {
        Ok(h) => h,
        Err(e) => {
            posix::warn_path(&prog, name, &posix::strerror(&e));
            std::process::exit(1);
        }
    };
    let mut g = Graph::default();
    let mut block = vec![0u8; 65536];
    loop {
        match posix::read_block(&mut *h, &mut block) {
            Ok(0) => break,
            Ok(n) => g.feed(&block[..n]),
            Err(e) => {
                posix::warn_path(&prog, name, &posix::strerror(&e));
                std::process::exit(1);
            }
        }
    }
    if !g.finish() {
        posix::warn_path(&prog, name, "odd data count");
        std::process::exit(1);
    }
    let (order, cycles) = sort(&mut g);
    let names = g.names();
    let mut out = Vec::with_capacity(65536 + 256);
    for &i in &order {
        out.extend_from_slice(&names[i]);
        out.push(b'\n');
        if out.len() >= 65536 {
            posix::write_stdout(&prog, &out);
            out.clear();
        }
    }
    posix::write_stdout(&prog, &out);
    for c in &cycles {
        posix::warn(&prog, "cycle in data");
        for &i in c {
            warn_item(&prog, &names[i]);
        }
    }
    std::process::exit(if cycles.is_empty() { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(chunks: &[&[u8]]) -> (Graph, bool) {
        let mut g = Graph::default();
        for c in chunks {
            g.feed(c);
        }
        let even = g.finish();
        (g, even)
    }

    /* names of the sorted items, and the number of cycles */
    fn tsort(chunks: &[&[u8]]) -> Option<(Vec<String>, usize)> {
        let (mut g, even) = graph(chunks);
        if !even {
            return None;
        }
        let (order, cycles) = sort(&mut g);
        let names = g.names();
        Some((order.iter().map(|&i| String::from_utf8_lossy(&names[i]).into_owned()).collect(), cycles.len()))
    }

    fn ts(s: &str) -> Vec<String> {
        let (o, c) = tsort(&[s.as_bytes()]).unwrap();
        assert_eq!(c, 0, "unexpected cycle in {s:?}");
        o
    }

    /* every pair "a b" has a before b */
    fn consistent(order: &[String], input: &str) -> bool {
        let t: Vec<&str> = input.split_whitespace().collect();
        t.chunks(2).all(|p| {
            let a = order.iter().position(|x| x == p[0]);
            let b = order.iter().position(|x| x == p[1]);
            a.is_some() && b.is_some() && (p[0] == p[1] || a < b)
        })
    }

    #[test]
    fn the_example_of_the_spec() {
        assert_eq!(ts("a b\nc c\nd e\ng g\nf g\ne f\nh h\n").join(" "), "a b c d e f g h");
    }

    #[test]
    fn empty_input_gives_nothing() {
        assert_eq!(ts(""), Vec::<String>::new());
        assert_eq!(ts(" \n\t\n"), Vec::<String>::new());
    }

    #[test]
    fn identical_items_only_declare_presence() {
        assert_eq!(ts("x x").join(" "), "x");
        assert_eq!(ts("b b\na a").join(" "), "b a");
    }

    #[test]
    fn a_chain_in_any_order() {
        assert_eq!(ts("c d\nb c\na b").join(" "), "a b c d");
        assert_eq!(ts("a b b c c d").join(" "), "a b c d");
    }

    #[test]
    fn the_output_follows_the_input_where_it_can() {
        assert_eq!(ts("z y\nx w").join(" "), "z y x w");
        assert_eq!(ts("b a\nd c").join(" "), "b a d c");
        assert_eq!(ts("a c\nb c").join(" "), "a b c");
    }

    #[test]
    fn repeated_pairs() {
        assert_eq!(ts("a b a b a b").join(" "), "a b");
        assert_eq!(ts("a c a c b c").join(" "), "a b c");
    }

    #[test]
    fn pairs_may_span_lines_and_any_white_space() {
        assert_eq!(ts("a\nb\nb\tc\x0bc\x0cd\re f").join(" "), "a b c d e f");
        assert_eq!(ts("  a   b  ").join(" "), "a b");
    }

    #[test]
    fn items_are_bytes() {
        let (o, c) = tsort(&[b"\xff \xfe \xfe a\x80"]).unwrap();
        assert_eq!(c, 0);
        assert_eq!(o.len(), 3);
        let (mut g, _) = graph(&[b"\xff \xfe"]);
        let names = g.names();
        assert_eq!(names, vec![b"\xff".to_vec(), b"\xfe".to_vec()]);
    }

    #[test]
    fn items_are_case_and_length_sensitive() {
        /* "a A" is a real pair, not a repeated item */
        assert_eq!(ts("a A\nA aa").join(" "), "a A aa");
        assert_eq!(ts("aa A\nA a").join(" "), "aa A a");
    }

    #[test]
    fn odd_data_is_an_error() {
        assert!(tsort(&[b"a"]).is_none());
        assert!(tsort(&[b"a b c"]).is_none());
        assert!(tsort(&[b"a b\nc\n"]).is_none());
        assert!(tsort(&[b"a b"]).is_some());
    }

    #[test]
    fn blocks_may_end_anywhere() {
        let input = "alpha beta\nbeta gamma\ngamma gamma\ndelta alpha\n";
        let want = ts(input);
        let b = input.as_bytes();
        for cut in 0..=b.len() {
            for cut2 in cut..=b.len() {
                let (o, c) = tsort(&[&b[..cut], &b[cut..cut2], &b[cut2..]]).unwrap();
                assert_eq!((o, c), (want.clone(), 0), "{cut} {cut2}");
            }
        }
        assert_eq!(want.join(" "), "delta alpha beta gamma");
    }

    #[test]
    fn a_cycle_is_reported_and_every_item_still_comes_out() {
        let (o, c) = tsort(&[b"a b\nb a\nc d\n"]).unwrap();
        assert_eq!(c, 1);
        let mut sorted = o.clone();
        sorted.sort();
        assert_eq!(sorted, ["a", "b", "c", "d"]);
        let pos = |x: &str| o.iter().position(|y| y == x).unwrap();
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn the_reported_cycle_lists_its_items_in_order() {
        let (mut g, _) = graph(&[b"x a\na b\nb c\nc a\nc y\n"]);
        let (order, cycles) = sort(&mut g);
        let names = g.names();
        assert_eq!(order.len(), 5);
        assert_eq!(cycles.len(), 1);
        let c: Vec<String> = cycles[0].iter().map(|&i| String::from_utf8_lossy(&names[i]).into_owned()).collect();
        assert_eq!(c.len(), 3);
        for i in 0..3 {
            let next = &c[(i + 1) % 3];
            let want = match c[i].as_str() {
                "a" => "b",
                "b" => "c",
                _ => "a",
            };
            assert_eq!(next, want, "{c:?}");
        }
    }

    #[test]
    fn two_cycles_and_a_loop_through_many() {
        let (o, c) = tsort(&[b"a b b a c d d c e f f g g e"]).unwrap();
        assert_eq!(c, 3);
        assert_eq!(o.len(), 7);
        let (o, c) = tsort(&[b"a b b c c d d a"]).unwrap();
        assert_eq!((o.len(), c), (4, 1));
    }

    #[test]
    fn a_cycle_downstream_of_a_chain() {
        let (o, c) = tsort(&[b"s a\na b\nb a\nb t\n"]).unwrap();
        assert_eq!(c, 1);
        assert_eq!(o.len(), 4);
        assert_eq!(o[0], "s");
    }

    #[test]
    fn a_pair_repeated_inside_a_cycle() {
        let (o, c) = tsort(&[b"a b a b b a"]).unwrap();
        assert_eq!(o.len(), 2);
        assert!(c >= 1);
    }

    #[test]
    fn a_big_chain_is_sorted_in_one_go() {
        let n = 200_000;
        let mut s = String::new();
        for i in (0..n).rev() {
            s.push_str(&format!("n{i} n{}\n", i + 1));
        }
        let o = ts(&s);
        assert_eq!(o.len(), n + 1);
        assert_eq!(o[0], "n0");
        assert_eq!(o[n], format!("n{n}"));
        assert!(o.windows(2).enumerate().all(|(i, w)| w[0] == format!("n{i}") && w[1] == format!("n{}", i + 1)));
    }

    #[test]
    fn many_small_cycles_stay_fast() {
        let mut s = String::new();
        for i in 0..20_000 {
            s.push_str(&format!("p{i} q{i} q{i} p{i}\n"));
        }
        let (o, c) = tsort(&[s.as_bytes()]).unwrap();
        assert_eq!((o.len(), c), (40_000, 20_000));
    }

    #[test]
    fn a_wide_fan() {
        let mut s = String::from("root ");
        s.push_str("leaf0\n");
        for i in 1..50_000 {
            s.push_str(&format!("root leaf{i}\n"));
        }
        let o = ts(&s);
        assert_eq!(o[0], "root");
        assert_eq!(o.len(), 50_001);
    }

    #[test]
    fn random_graphs_come_out_consistent() {
        /* a tiny linear congruential generator keeps this reproducible */
        let mut x: u64 = 12345;
        let mut next = move |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        for _ in 0..300 {
            let items = 2 + next(12);
            let mut s = String::new();
            for _ in 0..next(40) {
                /* edges only go from a smaller to a larger number: acyclic */
                let a = next(items);
                let b = next(items);
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                s.push_str(&format!("i{a} i{b}\n"));
            }
            let o = ts(&s);
            assert!(consistent(&o, &s), "{s:?} -> {o:?}");
        }
    }
}
