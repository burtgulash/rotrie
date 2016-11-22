use std::io::{BufReader,BufRead};
use std::iter::Iterator;
use std::{mem,str,slice};

const CHAR_BITS: usize = 8;
const SIZE_BITS: usize = CHAR_BITS;
const FIRSTS_BITS: usize = CHAR_BITS;
const ARE_TERMINAL_BITS: usize = 1;
const PTR_SIZES_BITS: usize = 2;
const TERM_LENS_BITS: usize = 4;


fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter())
            .take_while(|&(ac, bc)| ac == bc)
            .count()
}

struct BitWriter<'a> {
    target: &'a mut Vec<u8>,
    buf: u32,
    pos: usize,
}

impl<'a> BitWriter<'a> {
    fn new(target: &'a mut Vec<u8>) -> BitWriter<'a> {
        BitWriter {
            target: target,
            buf: 0,
            pos: 0,
        }
    }

    fn write(&mut self, size: usize, x: u32) -> usize {
        // println!("SIZE: {}, x: {}, MAX: {}", size, x, 1 << size);
        assert!(size <= 32);
        assert!(x < (1 << size));
        let written = self.flush();
        let mask = (1 << size) - 1;
        self.buf |= (x & mask) << (32 - size - self.pos);
        self.pos += size;
        written
    }

    fn flush(&mut self) -> usize {
        // println!("WRITE BUF: {:032b}", self.buf);
        let mut num_written = 0;
        while self.pos >= 8 {
            let byte = self.buf >> 24;
            self.buf <<= 8;
            self.pos -= 8;

            self.target.push(byte as u8);
            num_written += 1;
        }
        num_written
    }

    fn close(&mut self) -> usize {
        let mut num_written = self.flush();
        if self.pos > 0 {
            let byte = self.buf >> 24;
            self.target.push(byte as u8);
            num_written += 1
        }
        num_written
    }
}

struct BitReader<'a> {
    buf: u32,
    pos: usize,
    advanced_bytes: usize,
    bytes: &'a [u8],
}

impl<'a> BitReader<'a> {
    fn new(bytes: &[u8]) -> BitReader {
        BitReader {
            buf: 0,
            pos: 32,
            advanced_bytes: 0,
            bytes: bytes,
        }
    }

    fn advanced_by(&self) -> usize {
        self.advanced_bytes
    }

    fn fill(&mut self) {
        self.buf <<= 8;
        match self.bytes.split_first() {
            Some((&head, tail)) => {
                self.buf |= head as u32;
                self.bytes = tail;
                self.pos -= 8;
                self.advanced_bytes += 1;
            },
            None => {},
        };
    }

    fn read(&mut self, size: usize) -> u32 {
        while 32 - self.pos < size {
            self.fill();
            //while self.pos >= 8 {
            //}
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

struct TrieNode {
    prefix_len: usize,
    term_id: u32,
    term: Vec<u8>,
    ptr: usize,
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
    root_ptr: usize,
    ptr: usize,
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
        for ch in &mut node.children {
            let p = node.prefix_len + maxlen;
            assert!(p > node.prefix_len);
            if ch.prefix_len > p {
                let mut phantom = TrieNode::new(
                    p, 1338, ch.term[.. p].to_vec(), false
                );
                assert!(ch.prefix_len > phantom.prefix_len);
                mem::swap(ch, &mut phantom);
                ch.children.push(phantom);
                self.flush_children(ch);
            }
        }
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
        //println!("WRITING PTR: {:?}", &tmp);
        self.bytes.extend(tmp.iter());
    }

    fn flush_children(&mut self, node: &mut TrieNode) {
        if node.is_terminal {
            return
        }

        self.phantomize_children(node, 1 << TERM_LENS_BITS);
        self.root_ptr = self.ptr;
        node.ptr = self.ptr;
        println!("SETTING ROOT: {}", self.ptr);

        let node_size = node.children.len() as u32;
        assert!(node_size <= (1 << CHAR_BITS));


        let are_terminal: Vec<_> = node.children.iter().map(|ch| ch.is_terminal as u32).collect();
        let terms: Vec<_> = node.children.iter().map(|ch| &ch.term[node.prefix_len .. ch.prefix_len]).collect();
        let term_lens: Vec<_> = terms.iter().map(|cht| cht.len() as u32).collect();
        let firsts: Vec<_> = terms.iter().map(|ch| ch[0]).collect();
        let ptrs: Vec<u32> = node.children.iter().map(|ch| {
                if ch.is_terminal {
                    ch.term_id
                } else {
                    (node.ptr - ch.ptr) as u32
                    //ch.ptr as u32
                }
            }).collect();
        let ptr_sizes: Vec<_> = ptrs.iter().map(|&ptr| log2(ptr)).collect();

        // println!("FLUSH CHILDREN");
        // println!("size: {}", node_size);
        // println!("are_terminal: {:?}", &are_terminal);
        // println!("firsts: {:?}", &firsts);
        // println!("terms_lens: {:?}", &term_lens);
        // println!("ptr_sizes: {:?}", &ptr_sizes);
        // println!("terms: {:?}", &terms);
        // println!("ptrs: {:?}", ptrs);
        // // println!("NODEPTR: {}, NODESIZE: {}", node.ptr, node_size);
        // println!("...");

        {
            let mut ba = BitWriter::new(&mut self.bytes);
            self.ptr += ba.write(SIZE_BITS, node_size - 1);
            for &x in &are_terminal {
                self.ptr += ba.write(ARE_TERMINAL_BITS, x);
            }
            for &x in &firsts {
                self.ptr += ba.write(FIRSTS_BITS, x as u32);
            }
            for &x in &ptr_sizes {
                self.ptr += ba.write(PTR_SIZES_BITS, x - 1);
            }
            for &x in &term_lens {
                // -1 is for the 'first' character
                self.ptr += ba.write(TERM_LENS_BITS, x - 1);
            }
            for &x in &terms {
                for &c in &x[1..] {
                    self.ptr += ba.write(CHAR_BITS, c as u32);
                }
            }

            self.ptr += ba.close();
        }

        println!("END AT: {}", self.ptr);
        for &x in &ptrs {
            self.write_min_bytes(x);
        }
        println!("END AT: {}", self.ptr);
    }
}

struct Trie<'a> {
    bytes: &'a [u8],
    root_ptr: usize,
}

fn bs2x(bs: &[u8]) -> u32 {
    let x: &u32 = unsafe{mem::transmute(bs.as_ptr())};
    (*x).to_be()
}

fn bs2u32(n: usize, bs: &[u8]) -> &[u32] {
     unsafe {slice::from_raw_parts(
         mem::transmute(bs.as_ptr()), n
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
        for i in 0 .. n {
            x <<= 8;
            x |= self.bytes[ptr + i] as u32;
            // println!("ASSEMBLING BYTES: {} @ {}", self.bytes[ptr], ptr + i);
        }
        x
    }

    fn print(&self) {
        self.traverse(self.root_ptr, vec![]);
    }

    fn traverse(&self, ptr: usize, sofar: Vec<u8>) {
        let bs = &self.bytes[ptr..];
        let mut br = BitReader::new(bs);

        let size = br.read(SIZE_BITS) as usize + 1;
        let header_size_bits = SIZE_BITS + size * (ARE_TERMINAL_BITS + FIRSTS_BITS + PTR_SIZES_BITS + TERM_LENS_BITS);
        let header_size_bytes = (header_size_bits - 1) / 8 + 1;

        let mut are_terminal = Vec::new();
        for _ in 0 .. size {
            are_terminal.push(br.read(ARE_TERMINAL_BITS));
        }
        let mut firsts = Vec::new();
        for _ in 0 .. size {
            firsts.push(br.read(FIRSTS_BITS));
        }
        let mut ptr_sizes = Vec::new();
        for _ in 0 .. size {
            ptr_sizes.push(br.read(PTR_SIZES_BITS) + 1);
        }
        let mut term_lens = Vec::new();
        for _ in 0 .. size {
            term_lens.push(br.read(TERM_LENS_BITS));
        }
        let mut terms = Vec::new();
        for (&len, &first) in term_lens.iter().zip(firsts.iter()) {
            let mut term: Vec<u8> = sofar.clone();
                term.push('|' as u8);
            term.push(first as u8);
            for _ in 0 .. len {
                let x = br.read(CHAR_BITS) as u8;
                term.push(x);
            }
            terms.push(term);
        }


        // println!("\nPTR: {}", ptr);
        // println!("SIZE: {}", size);
        // println!("BYTES:");
        // for &b in bs {
        //     print!("{:08b}", b);
        // }
        // println!("");
        // //println!("TRIE BYTES: {:?}", bs);
        // println!("are_terminal: {:?}", &are_terminal);
        // // // println!("firsts: {:?}", &firsts);
        // println!("ptr_sizes: {:?}", &ptr_sizes);
        // println!("term_lens: {:?}", &term_lens);
        // println!("terms: {:?}", &terms);
        // println!("");

        let mut p = ptr + br.advanced_by();
        let mut ptrs = Vec::new();
        for (&is_terminal, &ptr_size) in are_terminal.iter().zip(ptr_sizes.iter()) {
            let x = self.assemble_bytes(p, ptr_size as usize);
            //println!("PTR FOUND: {}, n: {}", x, ptr_size);
            p += ptr_size as usize;

            if is_terminal == 1 {
                ptrs.push(x);
            } else {
                ptrs.push(ptr as u32 - x);
                //ptrs.push(x);
            }
        }

        // println!("ptrs: {:?}", &ptrs);

        for ((&is_terminal, &x), term) in are_terminal.iter().zip(ptrs.iter()).zip(terms.into_iter()) {
            if is_terminal == 1 {
                //println!("TERM: {:?}, sofar: {:?}", &term, &sofar);
                //println!("TERMINAL, id: {}, term: {}", x, &bs2str(&term));
                let word = unsafe{str::from_utf8_unchecked(&term)};
                println!("TERMINAL, id: {}, term: {}", x, word);
            } else {
                self.traverse(x as usize, term);
                //println!("-");
            }
        }
    }
}

fn bs2str(bs: &[u8]) -> String {
    let t: Vec<_> = bs[..bs.len() - 2].chunks(2).map(|chunk| chunk[1] << 4 | chunk[0]).collect();
    unsafe{str::from_utf8_unchecked(&t)}.to_owned()
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
        let mut toks = line.bytes().collect::<Vec<_>>();

        //println!("{}", line);
        //let mut toks = Vec::new();
        //for b in line.as_bytes() {
        //    toks.push(b & 0xf);
        //    toks.push(b >> 4);
        //}

        //let mut previous_0 = false;
        //for &x in &toks {
        //    if previous_0 && x == 0 {
        //        panic!("TO NESMI");
        //    }
        //    previous_0 = x == 0;
        //}
        //toks.push(0);
        toks.push(0);

        println!("INSERTING: {}", line);
        t.add(toks);
    }
    println!("FINISHING...");
    t.finish();

    println!("");
    println!("");
    println!("");
    println!("");
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

    println!("SIZE: {}", t.bytes.len());
}
