//! `scene(n):` blocks with their fields and `step:` blocks (design.md "scene", "step").
//! Field names are plain identifiers recognized only at the start of a line inside
//! the block, followed by `:`.

use super::{PResult, Parser};
use crate::ast::ExprKind;
use crate::diag::{Diagnostic, code};
use crate::lexer::{Keyword, Tok};
use crate::span::Span;
use crate::stmt::*;

#[derive(Clone, Copy, PartialEq)]
enum In {
    Scene,
    Step,
}

const SCENE_FIELDS: &str = "space, show, hide, clip, view, step:";
const STEP_FIELDS: &str = "show, set, animate, view, note";

impl<'a> Parser<'a> {
    /// `scene(1):` with indented fields, then `step:` blocks.
    pub(super) fn scene(&mut self) -> PResult<Stmt> {
        let start = self.peek_span();
        let (id, id_span) = self.scene_ref()?;
        self.expect(Tok::Colon, "':'")?;
        if !self.at(Tok::Newline) || self.peek_at(1) != Tok::Indent {
            self.diag(Diagnostic::new(id_span, code::SYNTAX, "scene 아래에 들여쓴 칸이 와야 함").suggest("scene(1):\n    space: 2D(x, y)\n    show: f"));
            return Err(());
        }
        self.bump();
        self.bump();
        let mut fields = vec![];
        let mut steps: Vec<StepDef> = vec![];
        self.block_lines(|p| {
            if p.at(Tok::Kw(Keyword::Step)) {
                steps.push(p.step_block()?);
                return Ok(());
            }
            let f = p.field(In::Scene)?;
            if !steps.is_empty() {
                p.diag(Diagnostic::new(f.key_span, code::SCENE, format!("scene 칸 '{}'는 step보다 앞에 써야 함", f.key)));
                return Ok(());
            }
            fields.push(f);
            Ok(())
        });
        Ok(Stmt::Scene(SceneDef { id, id_span, fields, steps, span: start.to(self.prev_span()) }))
    }

    fn step_block(&mut self) -> PResult<StepDef> {
        let head = self.bump().span;
        self.expect(Tok::Colon, "'step' 뒤에 ':'")?;
        if !self.at(Tok::Newline) || self.peek_at(1) != Tok::Indent {
            self.diag(Diagnostic::new(head, code::SYNTAX, "step: 아래에 들여쓴 칸이 와야 함").suggest("step:\n        show: +f"));
            return Err(());
        }
        self.bump();
        self.bump();
        let mut fields = vec![];
        self.block_lines(|p| {
            if p.at(Tok::Kw(Keyword::Step)) {
                let span = p.peek_span();
                p.diag(Diagnostic::new(span, code::SCENE, "step 안에 step을 둘 수 없음").suggest("앞 step과 같은 들여쓰기"));
                return Err(());
            }
            fields.push(p.field(In::Step)?);
            Ok(())
        });
        Ok(StepDef { fields, head, span: head.to(self.prev_span()) })
    }

    /// `scene(1)` or `scene(2D(x, y))`.
    pub(super) fn scene_ref(&mut self) -> PResult<(SceneRef, Span)> {
        let start = self.bump().span;
        self.expect(Tok::LParen, "'scene' 뒤에 '('")?;
        let r = if self.at(Tok::Int) && !self.space_ahead() {
            SceneRef::Number(self.positive_int("scene 번호")?)
        } else if self.space_ahead() {
            SceneRef::System(self.space()?)
        } else {
            return self.unexpected("scene 번호");
        };
        let end = self.expect(Tok::RParen, "')'")?;
        Ok((r, start.to(end)))
    }

    /// `1`, `12`: a positive integer literal.
    pub(super) fn positive_int(&mut self, what: &str) -> PResult<u32> {
        if !self.at(Tok::Int) {
            return self.unexpected(what);
        }
        let t = self.bump();
        match self.src[t.span.range()].parse::<u32>() {
            Ok(n) if (1..=9999).contains(&n) => Ok(n),
            _ => {
                self.diag(Diagnostic::new(t.span, code::SYNTAX, format!("{what}는 1에서 9999 사이 정수")));
                Err(())
            }
        }
    }

    /// At `2D(` / `3D(` / `polar(`?
    fn space_ahead(&self) -> bool {
        let t = self.toks.get(self.pos);
        let n = self.toks.get(self.pos + 1);
        match (t, n) {
            (Some(t), Some(n)) if t.tok == Tok::Int && n.tok == Tok::Ident && n.span.start == t.span.end => {
                &self.src[n.span.range()] == "D" && self.peek_at(2) == Tok::LParen
            }
            (Some(t), _) if t.tok == Tok::Ident => &self.src[t.span.range()] == "polar" && self.peek_at(1) == Tok::LParen,
            _ => false,
        }
    }

    /// `2D(x, y)`
    fn space(&mut self) -> PResult<Space> {
        let start = self.peek_span();
        let kind = if self.at(Tok::Int) {
            let n = self.bump();
            self.bump();
            format!("{}D", &self.src[n.span.range()])
        } else if self.at(Tok::Ident) && self.peek_text() == "polar" {
            self.bump();
            "polar".to_string()
        } else {
            return self.unexpected("좌표계(2D(x, y))");
        };
        self.expect(Tok::LParen, "'('")?;
        let axes = self.name_list()?;
        let end = self.expect(Tok::RParen, "')'")?;
        Ok(Space { kind, axes, span: start.to(end) })
    }

    fn field(&mut self, ctx: In) -> PResult<Field> {
        let key_span = self.peek_span();
        let list = if ctx == In::Scene { SCENE_FIELDS } else { STEP_FIELDS };
        if !self.at(Tok::Ident) {
            return self.unexpected(&format!("칸 이름({list})"));
        }
        let key = self.peek_text().to_string();
        self.bump();
        self.expect(Tok::Colon, &format!("'{key}' 뒤에 ':'"))?;
        let value = match (ctx, key.as_str()) {
            (In::Scene, "space") => FieldValue::Space(self.space()?),
            (In::Scene, "show" | "hide") => FieldValue::Names(self.name_list()?),
            (In::Scene, "clip" | "view") | (In::Step, "view") => FieldValue::Expr(self.expr()?),
            (In::Step, "show") => FieldValue::Toggles(self.toggle_list()?),
            (In::Step, "set") => FieldValue::Set(self.set_items()?),
            (In::Step, "animate") => FieldValue::Animate(self.animate_item()?),
            (In::Step, "note") => {
                let append = self.eat(Tok::Plus);
                FieldValue::Note { append, value: self.expr()? }
            }
            (In::Scene, "set" | "animate" | "note") => {
                self.diag(Diagnostic::new(key_span, code::SCENE_FIELD, format!("'{key}'는 step 안에서만 씀")).suggest(format!("step:\n        {key}: ...")));
                return Err(());
            }
            (In::Step, "space" | "hide" | "clip") => {
                self.diag(Diagnostic::new(key_span, code::SCENE_FIELD, format!("'{key}'는 scene 칸이라 step 안에서는 못 씀")).suggest("scene 블록의 step 앞에"));
                return Err(());
            }
            _ => {
                self.diag(Diagnostic::new(key_span, code::SCENE_FIELD, format!("칸 이름 '{key}'는 없음")).suggest(list));
                return Err(());
            }
        };
        let span = key_span.to(self.prev_span());
        self.expect_line_end()?;
        Ok(Field { key, key_span, value, span })
    }

    /// `+T1, -H1`
    fn toggle_list(&mut self) -> PResult<Vec<(bool, crate::ast::Ident, Span)>> {
        let mut v = vec![];
        loop {
            let on = match self.peek() {
                Tok::Plus => true,
                Tok::Minus => false,
                Tok::Ident | Tok::TexName => {
                    let name = self.peek_text().to_string();
                    return self.error_here(code::SYNTAX, "step의 show에는 켜기 +, 끄기 -를 붙임", Some(&format!("+{name}")));
                }
                _ => return self.unexpected("+이름 또는 -이름"),
            };
            let sign = self.bump().span;
            let (name, span) = self.name()?;
            v.push((on, name, sign.to(span)));
            if !self.eat(Tok::Comma) {
                return Ok(v);
            }
        }
    }

    /// `a = 2, b = -1`
    fn set_items(&mut self) -> PResult<Vec<SetItem>> {
        let mut v = vec![];
        loop {
            let (name, name_span) = self.name()?;
            self.expect(Tok::Eq, "'='")?;
            let value = self.expr()?;
            v.push(SetItem { name, name_span, value });
            if !self.eat(Tok::Comma) {
                return Ok(v);
            }
        }
    }

    /// `n = 4 -> 64`, then optionally `, 2s` and `, loop` or `, repeat`.
    fn animate_item(&mut self) -> PResult<AnimateItem> {
        let (name, name_span) = self.name()?;
        self.expect(Tok::Eq, "'='")?;
        let from = self.expr()?;
        self.expect(Tok::Arrow, "'->' (시작값 -> 끝값)")?;
        let to = self.expr()?;
        let mut item = AnimateItem { name, name_span, from, to, duration: None, looped: false, repeat: false };
        while self.eat(Tok::Comma) {
            let t = self.toks[self.pos];
            let next = self.toks.get(self.pos + 1).copied();
            match (t.tok, next) {
                (Tok::Int | Tok::Decimal, Some(n)) if n.tok == Tok::Ident && n.span.start == t.span.end && &self.src[n.span.range()] == "s" && item.duration.is_none() => {
                    self.bump();
                    self.bump();
                    item.duration = Some((self.src[t.span.range()].to_string(), t.span.to(n.span)));
                }
                (Tok::Ident, _) if self.peek_text() == "loop" && !item.looped => {
                    self.bump();
                    item.looped = true;
                }
                (Tok::Ident, _) if self.peek_text() == "repeat" && !item.repeat => {
                    self.bump();
                    item.repeat = true;
                }
                _ => return self.unexpected("재생 시간(2s), loop 또는 repeat"),
            }
        }
        // `n = 4 -> 64` written as one expression would have been caught above; a
        // string here is a common slip for note.
        if matches!(item.to.kind, ExprKind::Str { .. }) {
            return self.error_here(code::SYNTAX, "animate 끝값은 숫자", Some("n = 4 -> 64"));
        }
        Ok(item)
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse_program;
    use crate::ast::ExprKind;
    use crate::diag::code;
    use crate::stmt::*;

    fn scene(src: &str) -> SceneDef {
        let (s, d) = parse_program(src);
        assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
        match s.into_iter().next().unwrap() {
            Stmt::Scene(sc) => sc,
            s => panic!("not a scene: {s:?}"),
        }
    }

    fn errs(src: &str) -> Vec<&'static str> {
        parse_program(src).1.into_iter().map(|d| d.code).collect()
    }

    #[test]
    fn scene_block_with_fields() {
        let sc = scene("scene(1):\n    space: 2D(t, y)\n    show: f, H1\n    hide: H2\n    clip: 0 <= t <= 10\n    view: -5 <= t <= 5 and -3 <= y <= 3\n");
        assert_eq!(sc.id, SceneRef::Number(1));
        assert_eq!(sc.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), ["space", "show", "hide", "clip", "view"]);
        let FieldValue::Space(sp) = &sc.fields[0].value else { panic!() };
        assert_eq!(sp.key(), "2D(t, y)");
        assert!(matches!(&sc.fields[1].value, FieldValue::Names(n) if n.len() == 2 && n[1].0.key == "H1"));
        assert!(matches!(&sc.fields[4].value, FieldValue::Expr(e) if matches!(e.kind, ExprKind::And { .. })));
        assert!(sc.steps.is_empty());
    }

    #[test]
    fn steps_in_order() {
        let src = "scene(2):\n    space: 2D(x, y)\n    show: f\n    step:\n        show: +T1, -H1\n        set: a = 2, b = -1\n        note: \"{x == 2}에서의 접선\"\n    step:\n        animate: n = 4 -> 64, 2s, loop\n        view: -1 <= x <= 1\n        note: + m\n";
        let sc = scene(src);
        assert_eq!(sc.steps.len(), 2);
        let s1 = &sc.steps[0].fields;
        assert!(matches!(&s1[0].value, FieldValue::Toggles(t) if t.len() == 2 && t[0].0 && !t[1].0 && t[1].1.key == "H1"));
        assert!(matches!(&s1[1].value, FieldValue::Set(v) if v.len() == 2 && v[1].name.key == "b" && matches!(v[1].value.kind, ExprKind::Neg { .. })));
        assert!(matches!(&s1[2].value, FieldValue::Note { append: false, value } if matches!(value.kind, ExprKind::Str { .. })));
        let s2 = &sc.steps[1].fields;
        let FieldValue::Animate(a) = &s2[0].value else { panic!() };
        assert_eq!((a.name.key.as_str(), a.duration.as_ref().map(|d| d.0.as_str()), a.looped), ("n", Some("2"), true));
        assert!(matches!(&s2[2].value, FieldValue::Note { append: true, value } if matches!(value.kind, ExprKind::Name { .. })));
    }

    #[test]
    fn animate_options_are_optional() {
        let sc = scene("scene(1):\n    space: 2D(x, y)\n    step:\n        animate: a = 0 -> 1.5\n    step:\n        animate: a = 1.5 -> 0, loop\n    step:\n        animate: a = 0 -> 1, repeat, 3s\n");
        let FieldValue::Animate(a) = &sc.steps[0].fields[0].value else { panic!() };
        assert!(a.duration.is_none() && !a.looped && !a.repeat);
        let FieldValue::Animate(a) = &sc.steps[1].fields[0].value else { panic!() };
        assert!(a.duration.is_none() && a.looped && !a.repeat);
        let FieldValue::Animate(a) = &sc.steps[2].fields[0].value else { panic!() };
        assert!(a.duration.as_ref().map(|d| d.0.as_str()) == Some("3") && !a.looped && a.repeat);
    }

    #[test]
    fn field_names_are_not_reserved_elsewhere() {
        // `show`, `note`, `view` stay ordinary names outside scene blocks.
        let (s, d) = parse_program("show = \"a\"\nnote = 0 < x < 1\ndefine:\n    view(x) -> y: x\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn del_and_req_scene_targets() {
        let (s, d) = parse_program("del f, scene(2)\nreq(scene(1).step(3);; \"순서가 어색함\")\nreq(scene(1))\n");
        assert!(d.is_empty(), "{d:?}");
        assert!(matches!(&s[0], Stmt::Del { names, scenes, .. } if names.len() == 1 && scenes[0].0 == SceneRef::Number(2)));
        let Stmt::Req { scene: Some((SceneRef::Number(1), Some((3, _)), _)), slots, .. } = &s[1] else { panic!("{:?}", s[1]) };
        assert!(slots.items[0].is_none() && slots.items.len() == 3);
        assert!(matches!(&s[2], Stmt::Req { scene: Some((SceneRef::Number(1), None, _)), .. }));
        let (s, d) = parse_program("del scene(2D(x, y))\n");
        assert!(d.is_empty(), "{d:?}");
        assert!(matches!(&s[0], Stmt::Del { scenes, .. } if matches!(&scenes[0].0, SceneRef::System(sp) if sp.key() == "2D(x, y)")));
    }

    #[test]
    fn scene_errors() {
        // Field after a step, unknown field, step-only field at scene level.
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n    step:\n        show: +f\n    show: g\n"), vec![code::SCENE]);
        assert_eq!(errs("scene(1):\n    spcae: 2D(x, y)\n"), vec![code::SCENE_FIELD]);
        assert_eq!(errs("scene(1):\n    note: \"a\"\n"), vec![code::SCENE_FIELD]);
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n    step:\n        clip: 0 < x\n"), vec![code::SCENE_FIELD]);
        // Missing sign in a step show; bad animate tail; step outside a scene.
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n    step:\n        show: f\n"), vec![code::SYNTAX]);
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n    step:\n        animate: n = 4 -> 64, fast\n"), vec![code::SYNTAX]);
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n    step:\n        animate: n = 4 -> 64, repeat, repeat\n"), vec![code::SYNTAX]);
        assert_eq!(errs("step:\n    show: +f\n"), vec![code::SCENE]);
        // Scene number, missing block, nested step.
        assert_eq!(errs("scene(0):\n    space: 2D(x, y)\n"), vec![code::SYNTAX]);
        assert_eq!(errs("scene(1): space: 2D(x, y)\n"), vec![code::SYNTAX]);
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n    step:\n        step:\n            show: +f\n"), vec![code::SCENE]);
    }

    #[test]
    fn independent_errors_in_one_scene() {
        // Each bad line is reported; good lines around them still parse.
        let (s, d) = parse_program("scene(1):\n    space: 2D(x, y)\n    show f\n    view: -1 <= x <= 1\n    step:\n        set: a 2\n        note: \"ok\"\n");
        assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), vec![code::SYNTAX, code::SYNTAX]);
        let Stmt::Scene(sc) = &s[0] else { panic!() };
        assert_eq!(sc.fields.len(), 2);
        assert_eq!(sc.steps[0].fields.len(), 1);
    }
}
