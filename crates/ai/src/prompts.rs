//! The prompts, localized, with the JSON contract written out. The contract's keys (`slides`,
//! `title`, `bullets`, `body`, `index`) are never translated: the extractor and the window read
//! them by name.
//!
//! The answer's *text* is in the user's language. A rewrite keeps the slide's own language unless
//! the instruction asks for another one, so the answers do not drift per prompt.

/// One language's prompt set.
pub struct Prompts {
    pub system: &'static str,
    pub answer_in: &'static str,
    pub generate: &'static str,
    pub edit: &'static str,
    pub fix: &'static str,
}

/// The prompts for a language code (`pt`, `pt-br`, `es`, `ja`, ...): Portuguese, Spanish and
/// Japanese when we have them, English otherwise (the source language).
pub fn for_lang(lang: &str) -> &'static Prompts {
    match lang.split(['-', '_']).next().unwrap_or("en").to_ascii_lowercase().as_str() {
        "pt" => &PT,
        "es" => &ES,
        "ja" => &JA,
        _ => &EN,
    }
}

/// Fills `{name}` placeholders in one pass. Values are never re-expanded, so slide text that
/// happens to contain braces is safe.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(end) = after.find('}') else {
            out.push('{');
            rest = after;
            continue;
        };
        let name = &after[..end];
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

static EN: Prompts = Prompts {
    system: "You are a presentation assistant. Answer in {answer_in} unless the task asks for \
             another language. Return only the JSON object described: no prose, no markdown, no \
             code fences.",
    answer_in: "English",
    generate: "Build a presentation about: {topic}\n\n\
               Return one JSON object with exactly this shape:\n\
               {{\"slides\": [{{\"role\": \"title\", \"title\": \"the deck's title\", \"bullets\": []}}, {{\"role\": \"section\", \"title\": \"Part one\"}}, {{\"role\": \"content\", \"title\": \"a slide title\", \"bullets\": [\"point one\", \"\\ta sub-point\", \"point two\"]}}]}}\n\n\
               Rules:\n\
               - exactly {slides} slides;\n\
               - \"role\" is one of: title (the opening slide, no bullets), section (a divider before a part) or content (bullets);\n\
               - the first slide is the title role; every content slide has a title and 1 to 5 short bullets, one line each, without \
               numbering or bullet characters;\n\
               - a bullet that belongs under the one before it starts with a tab;\n\
               - the content is in {answer_in}.",
    edit: "Rewrite one slide of a presentation.\n\n\
           Instruction: {instruction}\n\n\
           Current slide (JSON):\n{slide}\n\n\
           Return one JSON object with exactly this shape:\n\
           {{\"title\": \"the new title\", \"body\": \"first bullet\\nsecond bullet\"}}\n\n\
           Rules:\n\
           - change only what the instruction asks;\n\
           - \"body\" holds the bullets, one per line, without numbering or bullet characters;\n\
           - keep the slide's language unless the instruction says otherwise.",
    fix: "Correct the spelling, grammar and punctuation of these slides. Keep the meaning, the \
          length and the language of each one.\n\n\
          Slides (JSON):\n{slides}\n\n\
          Return one JSON object with exactly this shape:\n\
          {{\"slides\": [{{\"index\": 0, \"title\": \"corrected title\", \"body\": \"corrected first bullet\\ncorrected second bullet\"}}]}}\n\n\
          Rules:\n\
          - one entry per slide, with the same index;\n\
          - \"body\" holds the bullets, one per line, without numbering;\n\
          - leave a slide out only when it needs no change.",
};

static PT: Prompts = Prompts {
    system: "Você é um assistente de apresentações. Responda em {answer_in} a menos que a tarefa \
             peça outro idioma. Devolva apenas o objeto JSON descrito: sem prosa, sem markdown, sem \
             cercas de código.",
    answer_in: "português (Brasil)",
    generate: "Monte uma apresentação sobre: {topic}\n\n\
               Devolva um objeto JSON exatamente com esta forma:\n\
               {{\"slides\": [{{\"role\": \"title\", \"title\": \"o título da apresentação\", \"bullets\": []}}, {{\"role\": \"section\", \"title\": \"Parte um\"}}, {{\"role\": \"content\", \"title\": \"título de um slide\", \"bullets\": [\"ponto um\", \"\\tsub-ponto\", \"ponto dois\"]}}]}}\n\n\
               Regras:\n\
               - exatamente {slides} slides;\n\
               - \"role\" é um de: title (o slide de abertura, sem tópicos), section (um divisor antes de uma parte) ou content (tópicos);\n\
               - o primeiro é o papel title; cada slide content tem um título e de 1 a 5 tópicos curtos, um por linha, sem numeração nem marcadores;\n\
               - um tópico que pertence ao anterior começa com uma tabulação;\n\
               - o conteúdo é em {answer_in}.",
    edit: "Reescreva um slide de uma apresentação.\n\n\
           Instrução: {instruction}\n\n\
           Slide atual (JSON):\n{slide}\n\n\
           Devolva um objeto JSON exatamente com esta forma:\n\
           {{\"title\": \"o novo título\", \"body\": \"primeiro tópico\\nsegundo tópico\"}}\n\n\
           Regras:\n\
           - mude só o que a instrução pede;\n\
           - \"body\" traz os tópicos, um por linha, sem numeração nem marcadores;\n\
           - mantenha o idioma do slide, a menos que a instrução peça outro.",
    fix: "Corrija a ortografia, a gramática e a pontuação destes slides. Mantenha o sentido, o \
          tamanho e o idioma de cada um.\n\n\
          Slides (JSON):\n{slides}\n\n\
          Devolva um objeto JSON exatamente com esta forma:\n\
          {{\"slides\": [{{\"index\": 0, \"title\": \"título corrigido\", \"body\": \"primeiro tópico corrigido\\nsegundo tópico corrigido\"}}]}}\n\n\
          Regras:\n\
          - uma entrada por slide, com o mesmo índice;\n\
          - \"body\" traz os tópicos, um por linha, sem numeração;\n\
          - deixe de fora só o slide que não precisa de mudança.",
};

static ES: Prompts = Prompts {
    system: "Eres un asistente de presentaciones. Responde en {answer_in} salvo que la tarea pida \
             otro idioma. Devuelve solo el objeto JSON descrito: sin prosa, sin markdown, sin \
             bloques de código.",
    answer_in: "español",
    generate: "Crea una presentación sobre: {topic}\n\n\
               Devuelve un objeto JSON exactamente con esta forma:\n\
               {{\"slides\": [{{\"role\": \"title\", \"title\": \"el título de la presentación\", \"bullets\": []}}, {{\"role\": \"section\", \"title\": \"Parte uno\"}}, {{\"role\": \"content\", \"title\": \"título de una diapositiva\", \"bullets\": [\"punto uno\", \"\\tsub-punto\", \"punto dos\"]}}]}}\n\n\
               Reglas:\n\
               - exactamente {slides} diapositivas;\n\
               - \"role\" es uno de: title (la de apertura, sin viñetas), section (un separador antes de una parte) o content (viñetas);\n\
               - la primera es el papel title; cada diapositiva content tiene un título y de 1 a 5 viñetas cortas, una por \
               línea, sin numeración ni viñetas;\n\
               - una viñeta que pertenece a la anterior empieza con una tabulación;\n\
               - el contenido está en {answer_in}.",
    edit: "Reescribe una diapositiva de una presentación.\n\n\
           Instrucción: {instruction}\n\n\
           Diapositiva actual (JSON):\n{slide}\n\n\
           Devuelve un objeto JSON exactamente con esta forma:\n\
           {{\"title\": \"el nuevo título\", \"body\": \"primera viñeta\\nsegunda viñeta\"}}\n\n\
           Reglas:\n\
           - cambia solo lo que pide la instrucción;\n\
           - \"body\" lleva las viñetas, una por línea, sin numeración ni viñetas;\n\
           - mantén el idioma de la diapositiva salvo que la instrucción pida otro.",
    fix: "Corrige la ortografía, la gramática y la puntuación de estas diapositivas. Mantén el \
          sentido, la extensión y el idioma de cada una.\n\n\
          Diapositivas (JSON):\n{slides}\n\n\
          Devuelve un objeto JSON exactamente con esta forma:\n\
          {{\"slides\": [{{\"index\": 0, \"title\": \"título corregido\", \"body\": \"primera viñeta corregida\\nsegunda viñeta corregida\"}}]}}\n\n\
          Reglas:\n\
          - una entrada por diapositiva, con el mismo índice;\n\
          - \"body\" lleva las viñetas, una por línea, sin numeración;\n\
          - deja fuera solo la diapositiva que no necesite cambios.",
};

static JA: Prompts = Prompts {
    system: "あなたはプレゼンテーションのアシスタントです。タスクが別の言語を求めない限り {answer_in} で答えてください。\
             説明した JSON オブジェクトだけを返してください: 文章も markdown もコードフェンスも不要です。",
    answer_in: "日本語",
    generate: "{topic} についてのプレゼンテーションを作成してください。\n\n\
               次の形の JSON オブジェクトを 1 つだけ返してください:\n\
               {{\"slides\": [{{\"role\": \"title\", \"title\": \"プレゼンテーションの見出し\", \"bullets\": []}}, {{\"role\": \"section\", \"title\": \"第 1 部\"}}, {{\"role\": \"content\", \"title\": \"スライドの見出し\", \"bullets\": [\"要点 1\", \"\\t副次的な要点\", \"要点 2\"]}}]}}\n\n\
               ルール:\n\
               - スライドはちょうど {slides} 枚。\n\
               - \"role\" は title (冒頭のスライド、要点なし)、section (部の前の区切り)、content (要点あり) のいずれか。\n\
               - 1 枚目は title。各 content スライドは見出しと 1〜5 個の短い要点、1 行に 1 つ、番号や記号は付けない。\n\
               - 直前の要点に属する要点は、先頭にタブを置く。\n\
               - 内容は {answer_in}。",
    edit: "プレゼンテーションの 1 枚のスライドを書き換えてください。\n\n\
           指示: {instruction}\n\n\
           現在のスライド (JSON):\n{slide}\n\n\
           次の形の JSON オブジェクトを 1 つだけ返してください:\n\
           {{\"title\": \"新しい見出し\", \"body\": \"要点 1\\n要点 2\"}}\n\n\
           ルール:\n\
           - 指示が求めたところだけを変える。\n\
           - \"body\" は要点を 1 行に 1 つ、番号や記号なし。\n\
           - 指示が別の言語を求めない限り、スライドの言語を保つ。",
    fix: "次のスライドの綴り・文法・句読点を直してください。それぞれの意味、長さ、言語は保ってください。\n\n\
          スライド (JSON):\n{slides}\n\n\
          次の形の JSON オブジェクトを 1 つだけ返してください:\n\
          {{\"slides\": [{{\"index\": 0, \"title\": \"修正後の見出し\", \"body\": \"修正後の要点 1\\n修正後の要点 2\"}}]}}\n\n\
          ルール:\n\
          - スライドごとに 1 エントリ、同じ index を使う。\n\
          - \"body\" は要点を 1 行に 1 つ、番号なし。\n\
          - 変更が不要なスライドだけ省略してよい。",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_the_json_contract() {
        for prompts in [&EN, &PT, &ES, &JA] {
            assert!(prompts.generate.contains("\"slides\""));
            assert!(prompts.generate.contains("\"bullets\""));
            assert!(prompts.generate.contains("\"role\""), "no role vocabulary in a generate prompt");
            assert!(prompts.edit.contains("\"body\""));
            assert!(prompts.fix.contains("\"index\""));
        }
    }

    #[test]
    fn the_language_pick_falls_back_to_english() {
        assert_eq!(for_lang("pt-br").answer_in, "português (Brasil)");
        assert_eq!(for_lang("pt").answer_in, "português (Brasil)");
        assert_eq!(for_lang("es").answer_in, "español");
        assert_eq!(for_lang("ja").answer_in, "日本語");
        assert_eq!(for_lang("de").answer_in, "English");
        assert_eq!(for_lang("").answer_in, "English");
    }

    #[test]
    fn placeholders_are_filled_once() {
        let text = fill("a {topic} b {slides} c", &[("topic", "{slides}"), ("slides", "3")]);
        assert_eq!(text, "a {slides} b 3 c", "a substituted value is not expanded again");
    }

    /// The JSON object's braces are not placeholders: they survive the fill untouched.
    #[test]
    fn the_json_contract_braces_survive() {
        let text = fill(EN.edit, &[("instruction", "shorter"), ("slide", "{}"), ("answer_in", "English")]);
        assert!(text.contains("{\"title\": \"the new title\", \"body\": \"first bullet\\nsecond bullet\"}"), "{text}");
        assert!(text.contains("shorter"));
    }
}
