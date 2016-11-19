use std::iter::Iterator;

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter())
            .take_while(|&(ac, bc)| ac == bc)
            .count()
}

struct BitWriter {
    buf: u32,
    pos: usize,
}

impl BitWriter {
    fn new() -> BitWriter {
        BitWriter {
            buf: 0,
            pos: 0,
        }
    }

    fn write(&mut self, to: &mut Vec<u8>, size: usize, x: u32) -> usize {
        let written = self.flush(to);
        let mask = (1 << size) - 1;
        self.buf |= (x & mask) << (32 - size - self.pos);
        self.pos += size;
        written
    }

    fn flush(&mut self, to: &mut Vec<u8>) -> usize {
        let mut num_written = 0;
        while self.pos >= 8 {
            let byte = self.buf >> 24;
            self.buf <<= 8;
            self.pos -= 8;

            to.push(byte as u8);
            num_written += 1;
        }
        num_written
    }

    fn close(&mut self, to: &mut Vec<u8>) -> usize {
        let mut num_written = self.flush(to);
        if self.pos > 0 {
            let byte = self.buf >> 24;
            to.push(byte as u8);
            num_written += 1
        }
        num_written
    }
}

struct BitReader<'a> {
    buf: u32,
    pos: usize,
    bytes: &'a [u8],
}

impl<'a> BitReader<'a> {
    fn new(bytes: &[u8]) -> BitReader {
        let mut br = BitReader {
            buf: 0,
            pos: 0,
            bytes: bytes,
        };
        br.fill();
        br.fill();
        br.fill();
        br.fill();
        br
    }

    fn fill(&mut self) {
        self.buf <<= 8;
        match self.bytes.split_first() {
            Some((&head, tail)) => {
                self.buf |= head as u32;
                self.bytes = tail;
            },
            None => {},
        };
    }

    fn read(&mut self, size: usize) -> u32 {
        while self.pos >= 8 {
            self.fill();
            self.pos -= 8;
        }
        println!("BUFF {:032b}", self.buf);
        let mask = (1 << size) - 1;
        let x = mask & (self.buf >> (32 - size - self.pos));
        self.pos += size;
        x
    }
}

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

// fn bitpack<I: Iterator<Item=u32>>(maxsize: usize, xs: I) -> u32 {
//     let mask = (1 << maxsize) - 1;
//     let mut target = 0u32;
//     for (i, x) in xs.enumerate() {
//         target |= (x & mask) << (32 - maxsize - i);
//     }
//     target.to_be()
// }
// 
// fn bitunpack(maxsize: usize, n: usize, bs: u32) -> Vec<u32> {
//     let mask = (1 << maxsize) - 1;
//     // let n = std::mem::size_of::<u32>() * 8 / maxsize;
//     let mut xs = Vec::new();
//     for i in 0 .. n {
//         let x = mask & (bs >> (32 - maxsize - i));
//         xs.push(x);
//     }
//     xs
// }
// 
// fn x2bs<'a, T>(x: &'a T) -> &'a [u8] {
//     let ptr: *const _ = unsafe{std::mem::transmute(x as *const _)};
//     let bs: &[u8] = unsafe{std::slice::from_raw_parts(ptr, std::mem::size_of::<T>())};
//     bs
// }


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
    term_id: u32,
    root_ptr: u32,
    ptr: u32,
}

impl TrieBuilder {
    fn new() -> TrieBuilder {
        TrieBuilder {
            stack: vec![TrieNode::new(0, 0, Vec::new(), false)],
            bytes: Vec::new(),
            term_id: 0,
            root_ptr: 0,
            ptr: 0,
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

    fn write_bits(&mut self, ba: &mut BitWriter, size: usize, x: u32) {
        self.ptr += ba.write(&mut self.bytes, size, x) as u32;
    }

    fn write_min_bytes(&mut self, mut x: u32) {
        loop {
            let byte = 0xf & x;
            self.bytes.push(byte as u8);
            self.ptr += 1;

            x >>= 8;
            if x == 0 {
                break
            }
        }
    }

    fn flush_children(&mut self, node: &mut TrieNode) {
        if node.is_terminal {
            return
        }

        self.phantomize_children(node, 16);
        node.ptr = self.ptr;

        let node_size = node.children.len() as u32;
        let are_terminal: Vec<_> = node.children.iter().map(|ch| ch.is_terminal as u32).collect();
        let terms: Vec<_> = node.children.iter().map(|ch| &ch.term[node.prefix_len .. ch.prefix_len]).collect();
        let term_lens: Vec<_> = terms.iter().map(|cht| cht.len() as u32).collect();
        let firsts: Vec<_> = terms.iter().map(|ch| *ch.iter().next().unwrap() as u32).collect();

        let ptrs: Vec<_> = node.children.iter().map(|ch| {
                if ch.is_terminal {
                    ch.term_id
                } else {
                    node.ptr - ch.ptr
                }
            }).collect();
        let ptr_sizes: Vec<_> = ptrs.iter().map(|&ptr| log2(ptr)).collect();

        println!("CHPTRS: {:?}", ptrs);
        println!("are_terminal: {:?}", &are_terminal);
        println!("NODEPTR: {}, NODESIZE: {}", node.ptr, node_size);

        let mut ba = BitWriter::new();
        self.write_bits(&mut ba, 4, node_size);
        for &x in &are_terminal {
            self.write_bits(&mut ba, 1, x);
        }
        for &x in &firsts {
            self.write_bits(&mut ba, 4, x);
        }
        for &x in &ptr_sizes {
            self.write_bits(&mut ba, 2, x);
        }
        for &x in &term_lens {
            self.write_bits(&mut ba, 2, x);
        }
        for &x in &terms {
            for &c in x {
                self.write_bits(&mut ba, 4, c as u32);
            }
        }
        self.ptr += ba.close(&mut self.bytes) as u32;

        for &x in &ptrs {
            self.write_min_bytes(x);
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

struct Trie<'a> {
    bytes: &'a [u8],
    root_ptr: usize,
}

fn bs2x(bs: &[u8]) -> u32 {
    let x: &u32 = unsafe{std::mem::transmute(bs.as_ptr())};
    (*x).to_be()
}

fn bs2u32(n: usize, bs: &[u8]) -> &[u32] {
     unsafe {std::slice::from_raw_parts(
         std::mem::transmute(bs.as_ptr()), n
     )}
}

impl<'a> Trie<'a> {
    fn new(bs: &'a [u8], root_ptr: usize) -> Trie<'a> {
        Trie {
            bytes: bs,
            root_ptr: root_ptr,
        }
    }

    fn print(&self) {
        self.traverse(self.root_ptr);
    }

    fn traverse(&self, ptr: usize) {
        let bs = &self.bytes[ptr..];
        //println!("BS: {:?}", bs);
        let mut br = BitReader::new(bs);
        let size = br.read(4);

        let mut are_terminal = Vec::new();
        for _ in 0 .. size {
            are_terminal.push(br.read(1));
        }
        let mut firsts = Vec::new();
        for _ in 0 .. size {
            firsts.push(br.read(4));
        }
        let mut ptr_sizes = Vec::new();
        for _ in 0 .. size {
            ptr_sizes.push(br.read(2));
        }
        let mut term_lens = Vec::new();
        for _ in 0 .. size {
            term_lens.push(br.read(2));
        }
        println!("SIZE: {}", size);
        println!("TRIE BYTES: {:?}", bs);
        println!("are_terminal: {:?}", &are_terminal);
        println!("firsts: {:?}", &firsts);
        println!("ptr_sizes: {:?}", &ptr_sizes);
        println!("term_lens: {:?}", &term_lens);

        // let size = br.read(&);
        // let size = bitunpack(4, 1, bs2x(bs))[0] as usize;
        // let are_terminal = bitunpack(1, size, bs2x(&bs[4..]));
        // let cht_lens = bitunpack(4, size, bs2x(&bs[8..]));
        // let ptrs = bs2u32(size, &bs[12..]);
        // let terms = bs2u32(size, &bs[12 + size * 4 ..]);

        // println!("SIZE: {}", size);
        // //println!("termos: {:?}", are_terminal);
        // //println!("chtlens: {:?}", cht_lens);
        // //println!("chts: {:?}", ptrs);
        // //println!("terms: {:?}", terms);

        // for (&is_terminal, &x) in are_terminal.iter().zip(ptrs.iter()) {
        //     if is_terminal == 1 {
        //         println!("TERMINAL, id: {}", x);
        //     } else {
        //         self.traverse(ptr - x as usize);
        //         println!("-");
        //     }
        // }
    }
}

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

    let mut bs: Vec<u8> = Vec::new();
    let mut ba = BitWriter::new();

    let nums = [14, 2, 5, 8, 0, 13, 2, 7, 7, 8];
    for &x in &nums {
        ba.write(&mut bs, 4, x);
    }

    ba.close(&mut bs);
    println!("NUMS: {:?}", &nums);
    println!("BA: {:?}", &bs);
    for x in &bs {
        print!("{:08b} ", x);
    }
    println!("");


    let trie = Trie::new(&t.bytes, t.root_ptr as usize);
    trie.print();
}
