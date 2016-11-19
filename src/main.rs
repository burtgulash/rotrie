use std::iter::Iterator;

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter())
            .take_while(|&(ac, bc)| ac == bc)
            .count()
}

// fn output<T, W: Write>(writer: &mut W, x: &T) -> std::io::Result<usize> {
//     let bytes = unsafe {std::slice::from_raw_parts(
//         std::mem::transmute(x),
//         std::mem::size_of::<T>(),
//     )};
//     writer.write(&bytes)
// }
// 
// fn output_slice<T, W: Write>(writer: &mut W, xs: &[T]) -> std::io::Result<usize>
// {
//     let bytes = unsafe {
//         std::slice::from_raw_parts(
//             std::mem::transmute(xs.as_ptr()),
//             xs.len() * std::mem::size_of::<T>(),
//         )
//     };
//     writer.write(&bytes)
// }
// 

fn log2(x: u32) -> u32 {
    if x > 0xffffff {
        4
    } else if x > 0xffff {
        3
    } else if x > 0xff {
        2
    } else {
        1
    }
}

fn bitpack<I: Iterator<Item=u32>>(maxsize: usize, xs: I) -> u32 {
    let mask = (1 << maxsize) - 1;
    let mut target = 0u32;
    for (i, x) in xs.enumerate() {
        target |= (x & mask) << (32 - maxsize - i);
    }
    target
}

fn x2bs<'a, T>(x: &'a T) -> &'a [u8] {
    let ptr: *const _ = unsafe{std::mem::transmute(x as *const _)};
    let bs: &[u8] = unsafe{std::slice::from_raw_parts(ptr, std::mem::size_of::<T>())};
    bs
}


struct TrieNode {
    prefix_len: usize,
    term_id: u32,
    term: Vec<u8>,
    ptr: u32,
    is_terminal: bool,
    children: Vec<TrieNode>,
}

impl TrieNode {
    fn new(prefix_len: usize, term_id: u32, term: Vec<u8>, is_terminal: bool) -> TrieNode {
        TrieNode {
            prefix_len: prefix_len,
            term_id: term_id,
            term: term,
            ptr: 0,
            is_terminal: is_terminal,
            children: Vec::new(),
        }
    }
}

struct TrieBuilder {
    stack: Vec<TrieNode>,
    bytes: Vec<u8>,
    root_ptr: u32,

    term_id: u32,
    ptr: u32,
    bsptr: u32,
    all_ptr: u32,
}

impl TrieBuilder {
    fn new() -> TrieBuilder {
        TrieBuilder {
            stack: vec![TrieNode::new(0, 0, Vec::new(), false)],
            root_ptr: 0,
            term_id: 0,
            ptr: 0,
            bsptr: 0,
            all_ptr: 0,
            bytes: Vec::new(),
        }
    }

    fn st_top(&self) -> usize {
        self.stack.len() - 1
    }

    fn add(&mut self, word: Vec<u8>) -> u32 {
        let (prefix_len, last_len) = {
            let l = self.st_top();
            let last_word = &self.stack[l].term;
            let prefix_len = common_prefix_len(&word, last_word);
            (prefix_len, last_word.len())
        };

        if prefix_len < last_len {
            let mut flushed = self.stack.pop().unwrap();
            while prefix_len < self.stack[self.st_top()].prefix_len {
                let mut parent = self.stack.pop().unwrap();
                parent.children.push(flushed);
                flushed = parent;

                self.flush_children(&mut flushed);
            }

            if prefix_len > self.stack[self.st_top()].prefix_len {
                self.stack.push(TrieNode::new(
                    prefix_len, 0, word[..prefix_len].to_vec(), false
                ));
            }

            let l = self.st_top();
            self.stack[l].children.push(flushed);
        }

        self.term_id += 1;
        self.stack.push(TrieNode::new(
            word.len(), self.term_id, word, true
        ));

        self.term_id
    }

    fn finish(&mut self) {
        self.add(vec![]);
        self.stack.pop().unwrap();

        let mut root = self.stack.pop().unwrap();
        self.root_ptr = self.ptr;
        self.flush_children(&mut root);
    }

    fn phantomize_children(&mut self, node: &mut TrieNode, maxlen: usize) {
        let mut children = std::mem::replace(&mut node.children, Vec::new());

        for ch in children.into_iter() {
            if ch.prefix_len - node.prefix_len <= maxlen {
                node.children.push(ch);
                continue
            }

            let ch_term = &ch.term[node.prefix_len .. ch.prefix_len];

            let mut p = ch.prefix_len - maxlen;
            let mut new_node = TrieNode::new(
                p, ch.term_id, ch.term[.. ch.prefix_len].to_vec(), true
            );
            println!("NEW NODE: {}, {},  {:?}", p, ch.term_id, &ch.term[.. ch.prefix_len]);

            loop {
                let lower_p = {
                    if p < node.prefix_len + maxlen {
                        node.prefix_len
                    } else {
                        p - maxlen
                    }
                };
                let mut phantom_node = TrieNode::new(
                    lower_p, 1337, ch.term[.. p].to_vec(), false
                );

                phantom_node.children.push(new_node);
                self.flush_children(&mut phantom_node);

                if lower_p <= node.prefix_len {
                    node.children.push(phantom_node);
                    break
                }

                new_node = phantom_node;
                p = lower_p;
            }
        }
    }

    fn write(&mut self, bs: &[u8]) {
        //let bs: &[u8] = unsafe{std::slice::from_raw_parts(xs.as_ptr() as *const _, xs.len() * std::mem::size_of::<T>())};
        self.bytes.extend(bs.iter());
        self.ptr += bs.len() as u32;
    }

    fn flush_children(&mut self, node: &mut TrieNode) {
        if node.is_terminal {
            return
        }

        self.phantomize_children(node, 16);
        node.ptr = self.ptr;

        let node_size = node.children.len() as u32;
        let mut header = 0u32;
        header |= node_size << (32 - 4);

        let terminal_mask = node.children.iter().map(|ch| ch.is_terminal as u32);
        let t_mask = bitpack(1, terminal_mask);

        let ch_terms: Vec<_> = node.children.iter().map(|ch| &ch.term[node.prefix_len .. ch.prefix_len]).collect();
        let cht_lens = ch_terms.iter().map(|cht| cht.len() as u32);
        let mut cht_lens_mask = bitpack(4, cht_lens);
        //println!("MASK: {:b}", cht_lens_mask);

        let ch_ptrs: Vec<_> = node.children.iter().map(|ch| {
                if ch.is_terminal {
                    ch.term_id
                } else {
                    node.ptr - ch.ptr
                }
            }).collect();
        println!("CHPTRS: {:?}", ch_ptrs);
        let ptr_sizes = ch_ptrs.iter().map(|&ptr| log2(ptr));
        let ptr_sizes_mask = bitpack(2, ptr_sizes);

        self.write(x2bs(&header));
        self.write(x2bs(&t_mask));
        self.write(x2bs(&ptr_sizes_mask));
        self.write(x2bs(&cht_lens_mask));
        for ch_ptr in &ch_ptrs {
            self.write(x2bs(ch_ptr));
        }
        for ch_term in &ch_terms {
            self.write(x2bs(ch_term));
        }
    }
}

// fn parse_line(line: &str) -> (Vec<u8>, u32, u32) {
//     let mut split = line.split("\t");
// 
//     let word = split.next().unwrap();
//     let mut word_bs = word.as_bytes().to_vec();
//     word_bs.push(0);
// 
//     let frequency = split.next().unwrap().parse::<u32>().unwrap();
//     let old_term_id = split.next().unwrap().parse::<u32>().unwrap();
// 
//     (word_bs, frequency, old_term_id)
// }

fn main() {
    let mut words = vec!["kokot", "kroketa", "kok", "kuk", "kokino", "kokinko"];
    words.sort();
    println!("WS: {:?}", &words);

    let mut t = TrieBuilder::new();
    for w in &words {
        let mut toks = Vec::new();
        for b in w.as_bytes() {
            toks.push(b & 0xf);
            toks.push(b >> 4);
        }
        toks.push(0);

        println!("toks: {:?}", toks);
        t.add(toks);
    }
    t.finish();
    println!("BYTES ({}): {:?}", t.bytes.len(), t.bytes);
    println!("ROOT AT: {}", t.root_ptr);
}

