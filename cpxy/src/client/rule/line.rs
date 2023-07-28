pub struct ColRowCounter<Iter> {
    i: Iter,
    pub line: usize,
    pub column: usize,
}

impl<Iter> ColRowCounter<Iter> {
    pub fn new(i: Iter) -> Self {
        Self {
            i,
            line: 0,
            column: 0,
        }
    }
}

impl<Iter: Iterator<Item = char>> Iterator for ColRowCounter<Iter> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.i.next();

        match result {
            Some('\n') => {
                self.line += 1;
                self.column = 0;
            }

            Some(_) => {
                self.column += 1;
            }

            _ => {}
        }

        result
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.i.size_hint()
    }
}
