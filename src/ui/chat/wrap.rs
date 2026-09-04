pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let w = word.chars().count();
            if w >= width {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                // Long word — chunk it.
                let mut buf = String::new();
                for ch in word.chars() {
                    if buf.chars().count() + 1 > width {
                        out.push(std::mem::take(&mut buf));
                    }
                    buf.push(ch);
                }
                if !buf.is_empty() {
                    out.push(buf);
                }
                continue;
            }
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + w <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

pub fn wrap_input_line(chars: &[char], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + width).min(chars.len());
        let end = if hard_end == chars.len() {
            hard_end
        } else {
            match chars[start..hard_end]
                .iter()
                .rposition(|c| c.is_whitespace())
            {
                Some(rel) => start + rel + 1,
                None => hard_end,
            }
        };
        chunks.push((start, end));
        start = end;
    }
    if chunks.is_empty() {
        chunks.push((0, 0));
    }
    chunks
}

pub fn count_visual_lines(text: &str, wrap_width: usize) -> usize {
    let w = wrap_width.max(1);
    let mut count = 0;
    for line in text.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        count += wrap_input_line(&chars, w).len();
    }
    count.max(1)
}
