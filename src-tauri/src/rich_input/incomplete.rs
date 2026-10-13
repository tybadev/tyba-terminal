//! O comando escrito na linha do TYBA está pela metade?
//!
//! Decide o que o Enter faz: comando completo vai para o shell como UMA
//! submissão; comando pela metade (`ls \`, aspas abertas, `cat <<EOF` sem
//! terminador, `|` no fim, `if` sem `fi`) ganha uma quebra de linha na caixa,
//! e nada sai para o PTY.
//!
//! Não é um parser de shell, e não tenta ser. A regra é assimétrica de
//! propósito: na dúvida, **completo**. Errar para "completo" custa pouco — o
//! shell recebe o comando, para no `PS2`, e a linha do TYBA segue como
//! continuação (PR #313); quem decide é o shell. Errar para "incompleto" prende
//! o usuário numa caixa que não envia. Por isso qualquer coisa que o scanner
//! não entende (um `fi` sem `if`, um `)` sobrando) devolve `false`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Frame {
    /// `$( … )`
    Subst,
    /// `( … )` — subshell, `$(( … ))`, `(( … ))`.
    Paren,
    /// `{ …; }` como grupo de comandos.
    Brace,
    /// `${ … }`
    Param,
    /// `" … "`
    DQuote,
    If,
    /// `for`/`while`/`until`/`select` até o `done`.
    Loop,
    Case,
}

/// Operador no fim do texto que obriga o shell a pedir mais linha.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TrailingOp {
    And,
    Or,
    Pipe,
}

/// `true` quando o shell pediria mais linha (`PS2`) para este texto.
pub fn is_incomplete(text: &str) -> bool {
    Scanner::new(text).run().unwrap_or(false)
}

struct Scanner {
    c: Vec<char>,
    i: usize,
    stack: Vec<Frame>,
    /// Delimitadores de heredoc esperando corpo: (delimitador, `<<-`).
    heredocs: Vec<(String, bool)>,
    /// A próxima palavra está em posição de comando (onde `if`, `fi`, `{`…
    /// são palavras reservadas).
    cmd_start: bool,
    word: String,
    /// A palavra corrente é só texto cru — sem aspas, `$` ou escape. Palavra
    /// reservada nunca é citada: `"if"` é argumento.
    plain: bool,
    in_word: bool,
    trailing: Option<TrailingOp>,
}

/// `None` = não sei; `Some(true)` = pela metade.
type Verdict = Option<bool>;

impl Scanner {
    fn new(text: &str) -> Self {
        Self {
            c: text.chars().collect(),
            i: 0,
            stack: Vec::new(),
            heredocs: Vec::new(),
            cmd_start: true,
            word: String::new(),
            plain: true,
            in_word: false,
            trailing: None,
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.c.get(self.i + offset).copied()
    }

    fn run(mut self) -> Verdict {
        while self.i < self.c.len() {
            match self.stack.last() {
                Some(Frame::DQuote) => self.in_dquote()?,
                Some(Frame::Param) => self.in_param()?,
                _ => {
                    if let Some(open) = self.in_code()? {
                        return Some(open);
                    }
                }
            }
        }
        self.finish_word()?;
        Some(!self.stack.is_empty() || !self.heredocs.is_empty() || self.trailing.is_some())
    }

    /// Entra em `$(`: lá dentro começa um comando novo, com palavra própria.
    /// A palavra de fora é retomada no `)` (ver `in_code`).
    fn enter_subst(&mut self) {
        self.stack.push(Frame::Subst);
        self.i += 2;
        self.cmd_start = true;
        self.in_word = false;
        self.plain = true;
        self.word.clear();
    }

    /// Começa (ou continua) uma palavra que já não pode ser reservada.
    fn mark_word(&mut self) {
        self.in_word = true;
        self.plain = false;
        self.trailing = None;
    }

    fn finish_word(&mut self) -> Option<()> {
        if !self.in_word {
            return Some(());
        }
        let word = std::mem::take(&mut self.word);
        let reserved = self.cmd_start && self.plain;
        self.in_word = false;
        self.plain = true;
        if !reserved {
            self.cmd_start = false;
            return Some(());
        }
        match word.as_str() {
            "if" => {
                self.stack.push(Frame::If);
                self.cmd_start = true;
            }
            "then" | "else" | "elif" | "do" | "!" => self.cmd_start = true,
            "while" | "until" => {
                self.stack.push(Frame::Loop);
                self.cmd_start = true;
            }
            "for" | "select" => {
                self.stack.push(Frame::Loop);
                self.cmd_start = false;
            }
            "case" => {
                self.stack.push(Frame::Case);
                self.cmd_start = false;
            }
            "{" => {
                self.stack.push(Frame::Brace);
                self.cmd_start = true;
            }
            "fi" => self.close(Frame::If)?,
            "done" => self.close(Frame::Loop)?,
            "esac" => self.close(Frame::Case)?,
            "}" => self.close(Frame::Brace)?,
            _ => self.cmd_start = false,
        }
        Some(())
    }

    /// Fecha o bloco do topo. Fechar o que não está aberto é sinal de que o
    /// scanner não entendeu o texto — e aí a resposta é "não sei".
    fn close(&mut self, frame: Frame) -> Option<()> {
        if self.stack.last() != Some(&frame) {
            return None;
        }
        self.stack.pop();
        self.cmd_start = false;
        Some(())
    }

    /// Pula `'…'` (a partir da aspa). `Err(())` quando não fecha.
    fn skip_single(&mut self, ansi_c: bool) -> Result<(), ()> {
        self.i += 1;
        while let Some(ch) = self.peek(0) {
            if ansi_c && ch == '\\' {
                self.i += 2;
                continue;
            }
            self.i += 1;
            if ch == '\'' {
                return Ok(());
            }
        }
        Err(())
    }

    /// Pula `` `…` `` (a partir da crase). `Err(())` quando não fecha.
    fn skip_backtick(&mut self) -> Result<(), ()> {
        self.i += 1;
        while let Some(ch) = self.peek(0) {
            if ch == '\\' {
                self.i += 2;
                continue;
            }
            self.i += 1;
            if ch == '`' {
                return Ok(());
            }
        }
        Err(())
    }

    fn in_dquote(&mut self) -> Option<()> {
        let ch = self.c[self.i];
        match ch {
            '\\' => self.i += 2,
            '"' => {
                self.stack.pop();
                self.i += 1;
            }
            '$' if self.peek(1) == Some('(') => {
                self.enter_subst();
            }
            '$' if self.peek(1) == Some('{') => {
                self.stack.push(Frame::Param);
                self.i += 2;
            }
            '`' => {
                if self.skip_backtick().is_err() {
                    // Crase aberta dentro de aspas abertas: as aspas já dizem
                    // que falta texto.
                    self.i = self.c.len();
                }
            }
            _ => self.i += 1,
        }
        Some(())
    }

    fn in_param(&mut self) -> Option<()> {
        let ch = self.c[self.i];
        match ch {
            '\\' => self.i += 2,
            '}' => {
                self.stack.pop();
                self.i += 1;
            }
            '"' => {
                self.stack.push(Frame::DQuote);
                self.i += 1;
            }
            '\'' => {
                if self.skip_single(false).is_err() {
                    self.i = self.c.len();
                }
            }
            '$' if self.peek(1) == Some('{') => {
                self.stack.push(Frame::Param);
                self.i += 2;
            }
            '$' if self.peek(1) == Some('(') => {
                self.enter_subst();
            }
            _ => self.i += 1,
        }
        Some(())
    }

    /// Um passo em contexto de código. `Some(Some(true))` encerra a varredura
    /// já com a resposta.
    fn in_code(&mut self) -> Option<Verdict> {
        let ch = self.c[self.i];
        match ch {
            ' ' | '\t' => {
                self.finish_word()?;
                self.i += 1;
            }
            '\n' => {
                self.finish_word()?;
                self.i += 1;
                self.cmd_start = true;
                if !self.heredocs.is_empty() && !self.read_heredoc_bodies() {
                    return Some(Some(true));
                }
            }
            '#' if !self.in_word => {
                while self.i < self.c.len() && self.c[self.i] != '\n' {
                    self.i += 1;
                }
            }
            '\\' => match self.peek(1) {
                // Barra no fim: o caso que motivou tudo isto.
                None => return Some(Some(true)),
                // Continuação de linha: some, e a palavra segue. Se não vem
                // mais nada depois dela, o shell ainda está esperando.
                Some('\n') => {
                    if self.i + 2 >= self.c.len() {
                        return Some(Some(true));
                    }
                    self.i += 2;
                }
                Some(_) => {
                    self.mark_word();
                    self.i += 2;
                }
            },
            '\'' => {
                self.mark_word();
                if self.skip_single(false).is_err() {
                    return Some(Some(true));
                }
            }
            '"' => {
                self.mark_word();
                self.stack.push(Frame::DQuote);
                self.i += 1;
            }
            '`' => {
                self.mark_word();
                if self.skip_backtick().is_err() {
                    return Some(Some(true));
                }
            }
            '$' => {
                self.mark_word();
                match self.peek(1) {
                    Some('(') => {
                        self.enter_subst();
                    }
                    Some('{') => {
                        self.stack.push(Frame::Param);
                        self.i += 2;
                    }
                    Some('\'') => {
                        self.i += 1;
                        if self.skip_single(true).is_err() {
                            return Some(Some(true));
                        }
                    }
                    _ => self.i += 1,
                }
            }
            '(' => {
                self.finish_word()?;
                self.stack.push(Frame::Paren);
                self.cmd_start = true;
                self.trailing = None;
                self.i += 1;
            }
            ')' => {
                self.finish_word()?;
                self.i += 1;
                self.trailing = None;
                match self.stack.last() {
                    Some(Frame::Subst) => {
                        // Fim de `$(…)`: a palavra de fora continua.
                        self.stack.pop();
                        self.in_word = true;
                        self.plain = false;
                        self.cmd_start = false;
                    }
                    Some(Frame::Paren) => {
                        self.stack.pop();
                        // `f() {` — depois do `)` vem o corpo da função.
                        self.cmd_start = true;
                    }
                    // Padrão de `case`: `a)`.
                    _ if self.stack.contains(&Frame::Case) => self.cmd_start = true,
                    _ => return None,
                }
            }
            ';' => {
                self.finish_word()?;
                self.i += if self.peek(1) == Some(';') { 2 } else { 1 };
                self.cmd_start = true;
                self.trailing = None;
            }
            '&' => {
                self.finish_word()?;
                match self.peek(1) {
                    Some('&') => {
                        self.i += 2;
                        self.trailing = Some(TrailingOp::And);
                        self.cmd_start = true;
                    }
                    // `&>arquivo`: redirecionamento, não operador.
                    Some('>') => {
                        self.i += 2;
                        self.trailing = None;
                    }
                    _ => {
                        self.i += 1;
                        self.trailing = None;
                        self.cmd_start = true;
                    }
                }
            }
            '|' => {
                self.finish_word()?;
                match self.peek(1) {
                    Some('|') => {
                        self.i += 2;
                        self.trailing = Some(TrailingOp::Or);
                    }
                    Some('&') => {
                        self.i += 2;
                        self.trailing = Some(TrailingOp::Pipe);
                    }
                    _ => {
                        self.i += 1;
                        self.trailing = Some(TrailingOp::Pipe);
                    }
                }
                self.cmd_start = true;
            }
            '<' => {
                self.finish_word()?;
                self.trailing = None;
                if self.peek(1) == Some('<') && self.peek(2) != Some('<') {
                    self.i += 2;
                    self.read_heredoc_delimiter()?;
                } else if self.peek(1) == Some('<') {
                    self.i += 3;
                } else {
                    self.i += 1;
                }
            }
            '>' => {
                self.finish_word()?;
                self.trailing = None;
                self.i += 1;
            }
            _ => {
                if !self.in_word {
                    self.in_word = true;
                }
                self.word.push(ch);
                self.trailing = None;
                self.i += 1;
            }
        }
        Some(None)
    }

    /// Lê o delimitador depois de `<<` / `<<-`, com ou sem aspas.
    fn read_heredoc_delimiter(&mut self) -> Option<()> {
        let strip_tabs = self.peek(0) == Some('-');
        if strip_tabs {
            self.i += 1;
        }
        while matches!(self.peek(0), Some(' ') | Some('\t')) {
            self.i += 1;
        }
        let mut delim = String::new();
        while let Some(ch) = self.peek(0) {
            match ch {
                ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')' => break,
                '\'' | '"' => {
                    self.i += 1;
                    while let Some(q) = self.peek(0) {
                        self.i += 1;
                        if q == ch {
                            break;
                        }
                        delim.push(q);
                    }
                }
                '\\' => {
                    self.i += 1;
                    if let Some(next) = self.peek(0) {
                        delim.push(next);
                        self.i += 1;
                    }
                }
                _ => {
                    delim.push(ch);
                    self.i += 1;
                }
            }
        }
        if delim.is_empty() {
            return None;
        }
        self.heredocs.push((delim, strip_tabs));
        self.cmd_start = false;
        Some(())
    }

    /// Consome os corpos dos heredocs pendentes, a partir do começo de linha.
    /// `false` quando o texto acaba antes de algum terminador.
    fn read_heredoc_bodies(&mut self) -> bool {
        let pending = std::mem::take(&mut self.heredocs);
        for (delim, strip_tabs) in pending {
            loop {
                if self.i >= self.c.len() {
                    return false;
                }
                let start = self.i;
                while self.i < self.c.len() && self.c[self.i] != '\n' {
                    self.i += 1;
                }
                let line: String = self.c[start..self.i].iter().collect();
                if self.i < self.c.len() {
                    self.i += 1;
                }
                let line = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if line == delim {
                    break;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::is_incomplete;

    fn incomplete(text: &str) {
        assert!(is_incomplete(text), "devia estar pela metade: {text:?}");
    }

    fn complete(text: &str) {
        assert!(!is_incomplete(text), "devia estar completo: {text:?}");
    }

    #[test]
    fn comando_simples_e_vazio_estao_completos() {
        complete("");
        complete("   ");
        complete("ls -la");
        complete("git commit -m 'msg'");
    }

    #[test]
    fn barra_no_fim() {
        incomplete("ls \\");
        incomplete("ls \\\n");
        complete("ls \\\n-la");
        // Barra escapada não continua nada.
        complete("echo \\\\");
        // Barra seguida de espaço escapa o espaço.
        complete("echo a\\ b");
    }

    #[test]
    fn aspas_simples() {
        incomplete("echo 'abc");
        complete("echo 'abc'");
        complete("echo 'it''s'");
        // Dentro de aspas simples a barra é literal.
        complete("echo 'a\\'");
        incomplete("echo 'a\nb");
        complete("echo 'a\nb'");
    }

    #[test]
    fn aspas_duplas() {
        incomplete("echo \"abc");
        complete("echo \"abc\"");
        complete("echo \"a \\\" b\"");
        incomplete("echo \"a \\\"");
        complete("echo \"$(echo \")\")\"");
    }

    #[test]
    fn crase() {
        incomplete("echo `date");
        complete("echo `date`");
    }

    #[test]
    fn ansi_c() {
        complete("echo $'it\\'s'");
        incomplete("echo $'abc");
    }

    #[test]
    fn substituicao_e_subshell() {
        incomplete("echo $(date");
        complete("echo $(date)");
        incomplete("(cd /tmp");
        complete("(cd /tmp && ls)");
        complete("echo $(( 1 + 2 ))");
        incomplete("echo $(( 1 + 2 )");
        complete("(( x++ ))");
        incomplete("echo ${HOME");
        complete("echo ${HOME:-/}");
        complete("echo ${x:-\"}\"}");
        // Dentro de `$(` começa comando novo, mesmo colado numa palavra.
        complete("echo a$(if true; then echo b; fi)c");
        incomplete("echo a$(if true; then echo b");
    }

    #[test]
    fn grupo_de_chaves() {
        incomplete("{ echo a;");
        complete("{ echo a; }");
        incomplete("f() {");
        complete("f() { echo a; }");
        // Expansão de chaves não é grupo.
        complete("echo {a,b}");
        complete("echo }");
    }

    #[test]
    fn heredoc() {
        incomplete("cat <<EOF");
        incomplete("cat <<EOF\nx");
        complete("cat <<EOF\nx\nEOF");
        complete("cat <<'EOF'\nx\nEOF");
        complete("cat <<\"EOF\"\n$x\nEOF");
        incomplete("cat <<-EOF\n\tx");
        complete("cat <<-EOF\n\tx\n\tEOF");
        // Sem `-`, o terminador indentado não fecha.
        incomplete("cat <<EOF\nx\n\tEOF");
        // Here-string não é heredoc.
        complete("cat <<< hi");
        // Dois heredocs na mesma linha, na ordem.
        incomplete("cat <<A <<B\na\nA");
        complete("cat <<A <<B\na\nA\nb\nB");
    }

    #[test]
    fn operador_no_fim() {
        incomplete("make &&");
        incomplete("make ||");
        incomplete("ls |");
        incomplete("ls | ");
        incomplete("ls |\n");
        incomplete("ls |&");
        complete("ls | grep a");
        complete("make && make test");
        complete("ls |\ngrep a");
        // `&` sozinho é segundo plano, não continuação.
        complete("sleep 1 &");
        complete("cmd &> out");
    }

    #[test]
    fn if_then_fi() {
        incomplete("if true; then");
        incomplete("if true; then echo a");
        incomplete("if true; then echo a; else");
        complete("if true; then echo a; fi");
        complete("if true; then echo a; elif false; then echo b; else echo c; fi");
        incomplete("if true\nthen\necho a");
        complete("if true\nthen\necho a\nfi");
    }

    #[test]
    fn lacos() {
        incomplete("for i in 1 2; do");
        complete("for i in 1 2; do echo $i; done");
        incomplete("while true; do sleep 1;");
        complete("while true; do sleep 1; done");
        incomplete("until false; do");
        complete("until false; do :; done");
        incomplete("for i in a b c\ndo\necho $i");
    }

    #[test]
    fn case_esac() {
        incomplete("case $x in");
        incomplete("case $x in a) echo a;;");
        complete("case $x in a) echo a;; esac");
        complete("case $x in (a) echo a;; *) echo b;; esac");
    }

    #[test]
    fn palavra_reservada_so_em_posicao_de_comando() {
        complete("echo if");
        complete("echo for done");
        complete("echo \"if\"");
        complete("grep -r then .");
    }

    #[test]
    fn comentarios_nao_contam() {
        complete("# comentário com \"aspas");
        complete("ls # e um | no fim");
        complete("ls # if");
        complete("echo 'a' # ' solta");
        // `#` no meio da palavra não é comentário.
        incomplete("echo a#b 'x");
    }

    #[test]
    fn na_duvida_esta_completo() {
        // Fechar o que não abriu: o scanner não entendeu, quem decide é o
        // shell.
        complete("fi");
        complete("done");
        complete("esac");
        complete("echo a)");
        complete("}");
        // Heredoc sem delimitador.
        complete("cat <<");
    }
}
