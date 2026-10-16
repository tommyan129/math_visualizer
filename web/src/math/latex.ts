// AST → LaTeX for MathJax (design.md "렌더 표기").
// - `*` is omitted on screen (· between two numbers); `/` is a vertical fraction,
//   inline inside exponents.
// - `()` (Paren) stays visible; groups written with `{}` get parentheses only where needed.
// - Function arguments are always parenthesized; positive integer powers of trig
//   functions render as sin²(x), other function powers as (f(x))².

import type { Expr } from "../generated/Expr";
import type { Ident } from "../generated/Ident";

export interface LatexCtx {
  /** CSS hex color per user function key, for coloring its name. */
  colors: Map<string, string>;
  /** Subexpression highlights: node id (Sum term ids included) → CSS hex color. */
  nodes?: Map<number, string>;
}

const TRIG = new Set(["sin", "cos", "tan", "sinh", "cosh", "tanh"]);
const BUILTIN_TEX: Record<string, string> = {
  sin: "\\sin",
  cos: "\\cos",
  tan: "\\tan",
  asin: "\\arcsin",
  acos: "\\arccos",
  atan: "\\arctan",
  sinh: "\\sinh",
  cosh: "\\cosh",
  tanh: "\\tanh",
  exp: "\\exp",
  ln: "\\ln",
  log10: "\\log_{10}",
  log2: "\\log_{2}",
  sign: "\\operatorname{sgn}",
  round: "\\operatorname{round}",
  gamma: "\\Gamma",
};

const BUILTIN_NAMES = new Set([...Object.keys(BUILTIN_TEX), "sqrt", "abs", "floor", "ceil", "log", "mod", "deg"]);

const SAFE_SUP = /^[A-Za-z0-9*+\-'(), ]*$/;
const HANGUL = /[ᄀ-ᇿ㄰-㆏가-힣]/;

function colored(tex: string, color: string | undefined): string {
  if (!color) return tex;
  // MathJax's color package knows rgb / RGB / gray models (not HTML).
  const n = parseInt(color.replace("#", ""), 16);
  return `\\textcolor[RGB]{${(n >> 16) & 255},${(n >> 8) & 255},${n & 255}}{${tex}}`;
}

function baseTex(base: string): string {
  if (base === "\\e") return "\\mathrm{e}";
  if (base === "\\i") return "\\mathrm{i}";
  if (base.startsWith("\\")) return base;
  if (HANGUL.test(base)) return `\\text{${base}}`;
  if (base.length === 1) return base;
  return `\\mathit{${base}}`;
}

export function identTex(id: Ident, color?: string): string {
  let t = baseTex(id.base);
  if (id.accent) t = `\\${id.accent}{${t}}`;
  t = colored(t, color);
  if (id.sub !== null) t += `_{${id.sub.split(/(\\[A-Za-z]+)/).map((p) => (p.startsWith("\\") ? p + " " : p)).join("")}}`;
  let sup = "";
  if (id.sup !== null) sup += SAFE_SUP.test(id.sup) ? id.sup : "\\ast";
  if (id.deriv > 0) sup += id.deriv <= 3 ? "\\prime".repeat(id.deriv) : `(${id.deriv})`;
  if (id.inverse) sup += "-1";
  if (sup) t += `^{${sup}}`;
  return t;
}

function numTex(text: string): string {
  const m = /^([0-9.]+)[eE]([+-]?)([0-9]+)$/.exec(text);
  if (!m) return text;
  const exp = (m[2] === "-" ? "-" : "") + m[3]!.replace(/^0+(?=\d)/, "");
  return `${m[1]}\\times 10^{${exp}}`;
}

const paren = (t: string) => `\\left(${t}\\right)`;

/** Strips visible parentheses where the structure already shows grouping. */
function unparen(e: Expr): Expr {
  return e.type === "Paren" ? unparen(e.inner) : e;
}

export function exprTex(e: Expr, ctx: LatexCtx, script = false): string {
  const t = nodeTex(e, ctx, script);
  return colored(t, ctx.nodes?.get(e.id));
}

const CMP_TEX = { Lt: "<", Le: "\\le", Gt: ">", Ge: "\\ge", Eq: "=" } as const;

function nodeTex(e: Expr, ctx: LatexCtx, script: boolean): string {
  const go = (c: Expr, s = script) => exprTex(c, ctx, s);
  switch (e.type) {
    case "Num":
      return numTex(e.text);
    case "Name":
      return identTex(e.name, e.kind === "Function" ? ctx.colors.get(e.name.key) : undefined);
    case "Sum":
      return e.terms
        .map((t, i) => {
          let s = go(t.expr);
          if (t.expr.type === "Sum" || (i > 0 && t.expr.type === "Neg")) s = paren(s);
          // A highlighted term is colored with its sign; \mathbin keeps binary spacing.
          const c = ctx.nodes?.get(t.id);
          if (i === 0) return c ? colored(t.sign === "Minus" ? `-${s}` : s, c) : t.sign === "Minus" ? `-${s}` : s;
          const op = t.sign === "Minus" ? "-" : "+";
          return c ? ` \\mathbin{${colored(op, c)}} ${colored(s, c)}` : ` ${op} ${s}`;
        })
        .join("");
    case "Product": {
      let out = "";
      e.factors.forEach((f, i) => {
        let s = go(f);
        if (f.type === "Sum" || (i > 0 && f.type === "Neg")) s = paren(s);
        if (i > 0) {
          // Keep a visible dot where juxtaposition would read as one number or is ambiguous.
          const startsWithDigit = /^[0-9.]/.test(s);
          out += startsWithDigit || f.type === "Div" && script ? " \\cdot " : " ";
        }
        out += s;
      });
      return out;
    }
    case "Div": {
      if (script) {
        const wrap = (c: Expr) => {
          const s = go(c, true);
          return c.type === "Sum" || c.type === "Product" || c.type === "Div" ? paren(s) : s;
        };
        return `${wrap(unparen(e.num))}/${wrap(unparen(e.den))}`;
      }
      return `\\frac{${go(unparen(e.num))}}{${go(unparen(e.den))}}`;
    }
    case "Pow": {
      const exp = go(unparen(e.exp), true);
      const b = e.base;
      if (b.type === "Call" && b.kind === "Builtin" && TRIG.has(b.callee.key)) {
        const ex = unparen(e.exp);
        if (ex.type === "Num" && ex.int && ex.text !== "0") return `${BUILTIN_TEX[b.callee.key]}^{${exp}}${paren(b.args.map((a) => go(unparen(a))).join(", "))}`;
      }
      let base = go(b);
      if (["Sum", "Product", "Div", "Neg", "Pow", "Factorial", "Call", "BigOp"].includes(b.type)) base = paren(base);
      return `{${base}}^{${exp}}`;
    }
    case "Neg": {
      const s = go(e.arg);
      return e.arg.type === "Sum" || e.arg.type === "Neg" ? `-${paren(s)}` : `-${s}`;
    }
    case "Factorial": {
      const s = go(e.arg);
      return ["Num", "Name", "Call", "Paren"].includes(e.arg.type) ? `${s}!` : `${paren(s)}!`;
    }
    case "Paren":
      return paren(go(e.inner));
    case "Call": {
      const key = e.callee.key;
      const args = e.args.map((a) => go(unparen(a)));
      // Slot ASTs kept as written are unresolved; builtins are known by name.
      if (e.kind === "Builtin" || (e.kind === "Unresolved" && BUILTIN_NAMES.has(key))) {
        switch (key) {
          case "sqrt":
            return `\\sqrt{${args[0]}}`;
          case "abs":
            return `\\left|${args[0]}\\right|`;
          case "floor":
            return `\\left\\lfloor ${args[0]}\\right\\rfloor`;
          case "ceil":
            return `\\left\\lceil ${args[0]}\\right\\rceil`;
          case "log":
            return `\\log_{${args[1]}}${paren(args[0]!)}`;
          case "mod": {
            const atom = (i: number) => ["Num", "Name", "Call", "Paren"].includes(unparen(e.args[i]!).type);
            return `${atom(0) ? args[0] : paren(args[0]!)} \\bmod ${atom(1) ? args[1] : paren(args[1]!)}`;
          }
          case "deg":
            return `${["Num", "Name"].includes(unparen(e.args[0]!).type) ? args[0] : paren(args[0]!)}^{\\circ}`;
        }
        return `${BUILTIN_TEX[key] ?? `\\operatorname{${key}}`}${paren(args.join(", "))}`;
      }
      return `${identTex(e.callee, ctx.colors.get(key))}${paren(args.join(", "))}`;
    }
    case "BigOp": {
      const op = e.op === "Sum" ? "\\sum" : "\\prod";
      const k = identTex(e.index);
      const lo = go(e.lo, true);
      const hi = go(e.hi, true);
      let body = go(e.body);
      if (e.body.type === "Sum") body = paren(body);
      const range = !e.lo_strict && !e.hi_strict ? `_{${k}=${lo}}^{${hi}}` : `_{${lo}${e.lo_strict ? "<" : "\\le "}${k}${e.hi_strict ? "<" : "\\le "}${hi}}`;
      return `${op}${range} ${body}`;
    }
    case "Compare": {
      let out = go(e.first);
      for (const l of e.rest) out += ` ${CMP_TEX[l.op]} ${go(l.rhs)}`;
      return out;
    }
    case "And":
      return e.args.map((a) => (a.type === "Or" ? paren(go(a)) : go(a))).join(" \\;\\text{and}\\; ");
    case "Or":
      return e.args.map((a) => go(a)).join(" \\;\\text{or}\\; ");
    case "Not":
      return `\\text{not}\\,${["Compare", "And", "Or"].includes(e.arg.type) ? paren(go(e.arg)) : go(e.arg)}`;
    case "Str":
      return `\\text{${e.raw.replace(/[\\{}$&#^_%~]/g, "")}}`;
    default:
      return "\\text{?}";
  }
}

/** `f(x) = body` as shown in the definition list (constants are not shown in the head). */
export function definitionTex(name: Ident, inputs: Ident[], body: Expr, ctx: LatexCtx): string {
  const head = `${identTex(name, ctx.colors.get(name.key))}${paren(inputs.map((i) => identTex(i)).join(", "))}`;
  return `${head} = ${exprTex(body, ctx)}`;
}
