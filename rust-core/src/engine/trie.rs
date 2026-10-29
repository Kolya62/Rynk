// SPDX-License-Identifier: GPL-3.0-or-later
//! Compact lowercase character trie over a word list.
//!
//! Each node keeps the total probability mass of the words below it (next-letter prediction for
//! the dynamic key grid) and the best word probability below it (pruning in the decoder).

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub struct Node {
    edge_start: u32,
    edge_len: u32,
    /// Word ending here (id in the owner's word list) or NONE
    pub word: u32,
    /// Sum of linear probabilities of all words in the subtree
    pub mass: f32,
    /// Best log10 probability in the subtree
    pub best: f32,
}

#[derive(Debug, Default)]
pub struct Trie {
    nodes: Vec<Node>,
    edges: Vec<(char, u32)>,
}

impl Trie {
    /// `words`: (lowercase spelling, id, log10 p). Duplicates keep the first id.
    pub fn build<'a>(words: impl Iterator<Item = (&'a str, u32, f32)>) -> Self {
        // Build with per-node child lists, then flatten into contiguous sorted edges
        struct B {
            kids: Vec<(char, u32)>,
            word: u32,
            mass: f32,
            best: f32,
        }
        let mut b = vec![B { kids: Vec::new(), word: NONE, mass: 0.0, best: f32::NEG_INFINITY }];
        for (w, id, logp) in words {
            let p = 10f32.powf(logp);
            let mut n = 0usize;
            b[0].mass += p;
            b[0].best = b[0].best.max(logp);
            for c in w.chars() {
                let next = match b[n].kids.iter().find(|k| k.0 == c) {
                    Some(k) => k.1 as usize,
                    None => {
                        b.push(B { kids: Vec::new(), word: NONE, mass: 0.0, best: f32::NEG_INFINITY });
                        let i = b.len() - 1;
                        b[n].kids.push((c, i as u32));
                        i
                    }
                };
                n = next;
                b[n].mass += p;
                b[n].best = b[n].best.max(logp);
            }
            if b[n].word == NONE {
                b[n].word = id;
            }
        }
        let mut nodes = Vec::with_capacity(b.len());
        let mut edges = Vec::with_capacity(b.len());
        for node in &mut b {
            node.kids.sort_unstable_by_key(|k| k.0);
            nodes.push(Node {
                edge_start: edges.len() as u32,
                edge_len: node.kids.len() as u32,
                word: node.word,
                mass: node.mass,
                best: node.best,
            });
            edges.extend_from_slice(&node.kids);
        }
        Trie { nodes, edges }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    pub fn root(&self) -> u32 {
        0
    }

    pub fn node(&self, n: u32) -> &Node {
        &self.nodes[n as usize]
    }

    pub fn children(&self, n: u32) -> &[(char, u32)] {
        let node = &self.nodes[n as usize];
        &self.edges[node.edge_start as usize..(node.edge_start + node.edge_len) as usize]
    }

    pub fn child(&self, n: u32, c: char) -> Option<u32> {
        let kids = self.children(n);
        kids.binary_search_by_key(&c, |k| k.0).ok().map(|i| kids[i].1)
    }

    /// Node reached by a lowercase prefix
    pub fn walk(&self, prefix: &str) -> Option<u32> {
        if self.nodes.is_empty() {
            return None;
        }
        prefix.chars().try_fold(0u32, |n, c| self.child(n, c))
    }

    /// P(next letter = c | prefix) for every continuation of `prefix`
    pub fn next_letters(&self, prefix: &str) -> Vec<(char, f32)> {
        let Some(n) = self.walk(prefix) else { return Vec::new() };
        let total = self.nodes[n as usize].mass.max(f32::MIN_POSITIVE);
        self.children(n).iter().map(|&(c, k)| (c, self.nodes[k as usize].mass / total)).collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_and_predict() {
        let t = Trie::build([("как", 0, -2.0), ("кот", 1, -3.0), ("да", 2, -2.5)].into_iter());
        assert_eq!(t.node(t.walk("как").unwrap()).word, 0);
        assert!(t.walk("кит").is_none());
        let next = t.next_letters("к");
        let a = next.iter().find(|x| x.0 == 'а').unwrap().1;
        let o = next.iter().find(|x| x.0 == 'о').unwrap().1;
        assert!(a > o && (a + o - 1.0).abs() < 1e-4);
    }
}
