use std::io::{BufReader,BufRead,BufWriter,Write};
use std::fs::File;
use std::str;

static USAGE: &'static str = "usage: maketrie #WORDS #OCCURRENCES #GROUPS TRIEDIR < INPUT";

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter())
            .take_while(|&(ac, bc)| ac == bc)
            .count()
}

fn output<T, W: Write>(writer: &mut W, x: &T) -> std::io::Result<usize> {
    let bytes = unsafe {std::slice::from_raw_parts(
        std::mem::transmute(x),
        std::mem::size_of::<T>(),
    )};
    writer.write(&bytes)
}

fn output_slice<T, W: Write>(writer: &mut W, xs: &[T]) -> std::io::Result<usize>
{
    let bytes = unsafe {
        std::slice::from_raw_parts(
            std::mem::transmute(xs.as_ptr()),
            xs.len() * std::mem::size_of::<T>(),
        )
    };
    writer.write(&bytes)
}

struct TrieNode {
    prefix_len: usize,
    term_id: u32,
    term: Vec<u8>,
    ptr: u32,
    is_terminal: bool,
    frequency: u32,
    children: Vec<TrieNode>,
    children_not_yet_tagged: Vec<(u32, u32)>,
}

impl TrieNode {
    fn new(prefix_len: usize, term_id: u32, term: Vec<u8>,
           is_terminal: bool, frequency: u32) -> TrieNode
    {
        TrieNode {
            prefix_len: prefix_len,
            term_id: term_id,
            term: term,
            ptr: 0,
            is_terminal: is_terminal,
            frequency: frequency,

            children: Vec::new(),
            children_not_yet_tagged: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct ForkNode {
    children_ptr: u32,
    children_bsptr: u32,
    num_children: u32,
}

struct TrieBuilder<W> {
    stack: Vec<TrieNode>,
    group_map: Vec<u16>,

    group_id: u16,
    term_id: u32,
    ptr: u32,
    bsptr: u32,
    all_ptr: u32,

    num_words: usize,
    num_occurrences: usize,
    threshold: u32,

    children_bsptrs: W,
    children_ptrs: W,
    forks: W,
    groups: W,
}

impl<W: Write> TrieBuilder<W> {
    fn new(num_groups: usize, num_words: usize, num_occurrences: usize, chibs: W, chips: W, forks: W, groups: W) -> TrieBuilder<W> {
        let threshold = num_occurrences / num_groups;
        println!("THRES: {}", threshold);
        TrieBuilder {
            stack: vec![TrieNode::new(0, 0, Vec::new(), false, 0)],
            group_map: vec![0; num_words + 1],

            num_words: num_words,
            num_occurrences: num_occurrences,
            threshold: threshold as u32,

            group_id: 0,
            term_id: 0,
            ptr: 0,
            bsptr: 0,
            all_ptr: 0,

            children_bsptrs: chibs,
            children_ptrs: chips,
            forks: forks,
            groups: groups,
        }
    }

    fn st_top(&self) -> usize {
        self.stack.len() - 1
    }

    fn add(&mut self, word: Vec<u8>, frequency: u32) -> u32 {
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
                    prefix_len, 0, word[..prefix_len].to_vec(), false, 0
                ));
            }

            let l = self.st_top();
            self.stack[l].children.push(flushed);
        }

        self.term_id += 1;
        self.stack.push(TrieNode::new(
            word.len(), self.term_id, word, true, frequency
        ));

        self.term_id
    }

    fn finish(&mut self) {
        self.add(vec![], 0);
        self.stack.pop().unwrap();

        let mut root = self.stack.pop().unwrap();
        self.flush_children(&mut root);
        self.create_group(&mut root);

        output_slice(&mut self.groups, &self.group_map).unwrap();

        // println!("Num groups: {}", self.group_id);
    }

    fn flush_children(&mut self, node: &mut TrieNode) {
        if node.is_terminal {
            return
        }

        let bsptr = self.bsptr;
        node.ptr = self.ptr;
        self.ptr += 1;

        let all_ptr = self.all_ptr;
        self.all_ptr += node.children.len() as u32;

        let threshold = self.threshold;
        self.assign_group(node, threshold);

        for ch in &node.children {
            let ch_term = &ch.term[node.prefix_len .. ch.prefix_len];

            self.bsptr += ch_term.len() as u32 + 1;
            self.children_bsptrs.write(ch_term).unwrap();
            self.children_bsptrs.write(b" ").unwrap();

            if ch.is_terminal {
                output(&mut self.children_ptrs, &ch.term_id).unwrap();
            } else {
                output(&mut self.children_ptrs, &ch.ptr).unwrap();
            }

            //println!("Flush node '{}', {}, {}",
            //    unsafe{str::from_utf8_unchecked(ch_term)},
            //    ch_term.len(), ch.term_id);
        }

        println!("FLUSHING {}", bsptr);
        output(&mut self.forks, &ForkNode {
            children_ptr: all_ptr,
            children_bsptr: bsptr,
            num_children: node.children.len() as u32,
        }).unwrap();
    }

    fn assign_group(&mut self, node: &mut TrieNode, threshold: u32) {
        let mut size_of_unprocessed_lists = 0;
        for ch in &mut node.children {
            // Add this child node to either
            // 1. new list group
            if ch.frequency > threshold {
                self.create_group(ch);
            }

            // 2. to parent's group buffer
            // it will be assigned group later
            else {
                size_of_unprocessed_lists += ch.frequency;
                node.children_not_yet_tagged
                    .extend_from_slice(&ch.children_not_yet_tagged);
                if ch.is_terminal {
                    node.children_not_yet_tagged.push((ch.term_id, ch.frequency));
                }
            }
        }

        node.frequency += size_of_unprocessed_lists;
    }

    fn create_group(&mut self, node: &mut TrieNode) {
        self.group_id += 1;
        let mut g_terms = Vec::new();

        if node.is_terminal {
            g_terms.push((node.term_id, node.frequency));
        }
        for (term_id, frequency) in node.children_not_yet_tagged.drain(..) {
            g_terms.push((term_id, frequency));
        }

        // Frequency sort
        // g_terms.sort_by(|a, b| -a.1.cmp(&b.1));
        for (term_id, _) in g_terms.drain(..) {
            self.group_map[term_id as usize] = self.group_id;
        }
    }
}

fn parse_line(line: &str) -> (Vec<u8>, u32, u32) {
    let mut split = line.split("\t");

    let word = split.next().unwrap();
    let mut word_bs = word.as_bytes().to_vec();
    word_bs.push(0);

    let frequency = split.next().unwrap().parse::<u32>().unwrap();
    let old_term_id = split.next().unwrap().parse::<u32>().unwrap();

    (word_bs, frequency, old_term_id)
}

fn create_writer(dir: &str, filename: &str) -> BufWriter<File> {
    let path = format!("{}/{}", dir, filename);
    let file = File::create(path).unwrap();
    BufWriter::new(file)
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        println!("{}", USAGE);
        std::process::exit(1);
    }

    let num_words = (&args[1].trim()).parse::<usize>().unwrap();
    let num_occurrences = (&args[2].trim()).parse::<usize>().unwrap();
    let num_groups = (&args[3].trim()).parse::<usize>().unwrap();
    let trie_dir = &args[4];

    std::fs::DirBuilder::new()
        .recursive(true)
        .create(trie_dir).unwrap();

    let inreader = BufReader::new(std::io::stdin());
    let mut trie_builder = TrieBuilder::new(
        num_groups, num_words, num_occurrences,
        create_writer(trie_dir, "chibs"),
        create_writer(trie_dir, "chips"),
        create_writer(trie_dir, "forks"),
        create_writer(trie_dir, "groups"),
    );

    let mut tidmap_w = create_writer(trie_dir, "tidmap");
    let mut term_id_map = vec![0; num_words + 1];


    for line in inreader.lines() {
        let (word, frequency, old_term_id) = parse_line(&line.unwrap());
        let new_term_id = trie_builder.add(word, frequency);
        term_id_map[old_term_id as usize] = new_term_id;

        //println!("WORD {} - {}, {}", word, frequency, old_term_id);
    }

    trie_builder.finish();
    output_slice(&mut tidmap_w, &term_id_map).unwrap();
}
