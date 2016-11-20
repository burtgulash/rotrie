use std::io::{BufReader,BufRead};
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
        // println!("WRITE BUF: {:032b}", self.buf);
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
        if 32 - self.pos < size {
            while self.pos >= 8 {
                self.fill();
                self.pos -= 8;
            }
        }

        // println!("BUFF {:032b} @ {}", self.buf, self.pos);
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
            stack: vec![TrieNode::new(0, 0, vec![], false)],
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
        self.flush_children(&mut root);
    }

    fn phantomize_children(&mut self, node: &mut TrieNode, maxlen: usize) {
        let children = std::mem::replace(&mut node.children, Vec::new());

        for mut ch in children.into_iter() {
            let p = node.prefix_len + maxlen - 1;
            if ch.prefix_len >= p {
                let mut phantom = TrieNode::new(
                    p, 1338, ch.term[.. p].to_vec(), false
                );
                phantom.children.push(ch);
                self.flush_children(&mut phantom);

                // println!("PHANTOM NODE: {}, {:?}", p, &phantom.term);
                ch = phantom;
            }
            node.children.push(ch);

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
        let mut tmp = Vec::new();
        loop {
            let byte = 0xff & x;
            tmp.push(byte as u8);
            self.ptr += 1;

            x >>= 8;
            if x == 0 {
                break
            }
        }
        tmp.reverse();
        self.bytes.extend(tmp.iter());
    }

    fn flush_children(&mut self, node: &mut TrieNode) {
        if node.is_terminal {
            return
        }

        self.phantomize_children(node, 16);
        self.root_ptr = self.ptr;
        node.ptr = self.ptr;

        let node_size = node.children.len() as u32;
        let are_terminal: Vec<_> = node.children.iter().map(|ch| ch.is_terminal as u32).collect();
        let terms: Vec<_> = node.children.iter().map(|ch| &ch.term[node.prefix_len .. ch.prefix_len]).collect();
        let term_lens: Vec<_> = terms.iter().map(|cht| cht.len() as u32).collect();
        let firsts: Vec<_> = terms.iter().map(|ch| *ch.iter().next().unwrap_or(&0) as u32).collect();

        let ptrs: Vec<_> = node.children.iter().map(|ch| {
                if ch.is_terminal {
                    ch.term_id
                } else {
                    node.ptr - ch.ptr
                }
            }).collect();
        let ptr_sizes: Vec<_> = ptrs.iter().map(|&ptr| log2(ptr)).collect();

        // println!("FLUSH CHILDREN");
        // println!("are_terminal: {:?}", &are_terminal);
        // println!("firsts: {:?}", &firsts);
        // println!("terms_lens: {:?}", &term_lens);
        // println!("ptr_sizes: {:?}", &ptr_sizes);
        // println!("terms: {:?}", &terms);
        // println!("CHPTRS: {:?}", ptrs);
        // // println!("NODEPTR: {}, NODESIZE: {}", node.ptr, node_size);
        // println!("...");

        let mut ba = BitWriter::new();
        self.write_bits(&mut ba, 4, node_size - 1);
        for &x in &are_terminal {
            self.write_bits(&mut ba, 1, x);
        }
        // for &x in &firsts {
        //     self.write_bits(&mut ba, 4, x - 1);
        // }
        for &x in &ptr_sizes {
            self.write_bits(&mut ba, 2, x - 1);
        }
        for &x in &term_lens {
            self.write_bits(&mut ba, 4, x);
        }

        self.ptr += ba.close(&mut self.bytes) as u32;
        for &x in &ptrs {
            self.write_min_bytes(x);
        }

        let mut ba = BitWriter::new();
        for &x in &terms {
            for &c in x {
                self.write_bits(&mut ba, 4, c as u32);
            }
        }
        self.ptr += ba.close(&mut self.bytes) as u32;
    }
}

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

    fn assemble_bytes(&self, ptr: usize, n: usize) -> u32 {
        let mut x = 0;
        for _ in 0 .. n {
            x <<= 8;
            x |= self.bytes[ptr] as u32;
        }
        x
    }

    fn print(&self) {
        self.traverse(self.root_ptr, vec![]);
    }

    fn traverse(&self, ptr: usize, sofar: Vec<u8>) {
        let bs = &self.bytes[ptr..];
        let mut br = BitReader::new(bs);

        let SIZE_BITS = 4;
        let ARE_TERMINAL_BITS = 1;
        //let FIRSTS_BITS = 4;
        let FIRSTS_BITS = 0;
        let PTR_SIZES_BITS = 2;
        let TERM_LENS_BITS = 4;

        let size = br.read(SIZE_BITS) as usize + 1;
        let header_size_bits = SIZE_BITS + size * (ARE_TERMINAL_BITS + FIRSTS_BITS + PTR_SIZES_BITS + TERM_LENS_BITS);
        let header_size_bytes = (header_size_bits - 1) / 8 + 1;

        let mut are_terminal = Vec::new();
        for _ in 0 .. size {
            are_terminal.push(br.read(ARE_TERMINAL_BITS));
        }
        // let mut firsts = Vec::new();
        // for _ in 0 .. size {
        //     firsts.push(br.read(4) + 1);
        // }
        let mut ptr_sizes = Vec::new();
        for _ in 0 .. size {
            ptr_sizes.push(br.read(PTR_SIZES_BITS) + 1);
        }
        let mut term_lens = Vec::new();
        for _ in 0 .. size {
            term_lens.push(br.read(TERM_LENS_BITS));
        }

        let mut p = ptr + header_size_bytes as usize;
        let mut ptrs = Vec::new();
        for (&is_terminal, &ptr_size) in are_terminal.iter().zip(ptr_sizes.iter()) {
            let x = self.assemble_bytes(p, ptr_size as usize);
            p += ptr_size as usize;

            if is_terminal == 1 {
                ptrs.push(x);
            } else {
                ptrs.push(ptr as u32 - x);
            }
        }

        let mut br = BitReader::new(&self.bytes[p..]);
        let mut terms = Vec::new();
        for &len in &term_lens {
            let mut term: Vec<u8> = sofar.clone();
            for _ in 0 .. len {
                let x = br.read(4) as u8;
                term.push(x);
            }
            terms.push(term);
        }


        println!("SIZE: {}", size);
        println!("TRIE BYTES: {:?}", bs);
        println!("are_terminal: {:?}", &are_terminal);
        // println!("firsts: {:?}", &firsts);
        println!("ptr_sizes: {:?}", &ptr_sizes);
        println!("ptrs: {:?}", &ptrs);
        println!("term_lens: {:?}", &term_lens);
        println!("terms: {:?}", &terms);
        println!("---");

        for ((&is_terminal, &x), term) in are_terminal.iter().zip(ptrs.iter()).zip(terms.into_iter()) {
            if is_terminal == 1 {
                println!("TERM: {:?}, sofar: {:?}", &term, &sofar);
                println!("TERMINAL, id: {}, term: {}", x, &bs2str(&term));
            } else {
                self.traverse(x as usize, term);
                //println!("-");
            }
        }
    }
}

fn bs2str(bs: &[u8]) -> String {
    let t: Vec<_> = bs[..bs.len() - 2].chunks(2).map(|chunk| chunk[1] << 4 | chunk[0]).collect();
    unsafe{std::str::from_utf8_unchecked(&t)}.to_owned()
}


fn main() {
    //let mut words = vec!["kokot", "kroketa", "kok", "kuk", "kokino", "kokinko"];
    //words.sort();
    //println!("WS: {:?}", &words);

    let mut t = TrieBuilder::new();
    let stdin = BufReader::new(std::io::stdin());
    for line in stdin.lines() {
        let line = line.unwrap();
        let line = line.trim();

        println!("{}", line);
        let mut toks = Vec::new();
        for b in line.as_bytes() {
            toks.push(b & 0xf);
            toks.push(b >> 4);
        }
        toks.push(0);
        toks.push(0);

        println!("toks: {:?}", toks);
        t.add(toks);
    }
    println!("FINISHING...");
    t.finish();

    //println!("BYTES ({}): {:?}", t.bytes.len(), t.bytes);
    println!("BYTES ({})", t.bytes.len());
    println!("ROOT AT: {}", t.root_ptr);

    // let mut bs: Vec<u8> = Vec::new();
    // let mut ba = BitWriter::new();

    // let nums = [14, 2, 5, 8, 0, 13, 2, 7, 7, 8];
    // for &x in &nums {
    //     ba.write(&mut bs, 4, x);
    // }

    // ba.close(&mut bs);
    // println!("NUMS: {:?}", &nums);
    // println!("BA: {:?}", &bs);
    // for x in &bs {
    //     print!("{:08b} ", x);
    // }
    // println!("");


    let trie = Trie::new(&t.bytes, t.root_ptr as usize);
    trie.print();
}
