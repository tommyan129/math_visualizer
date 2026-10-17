// AST → LaTeX for MathJax (design.md "렌더 표기").
// - `*` is omitted on screen (· between two numbers); `/` is a vertical fraction,
//   inline inside exponents.
// - `()` (Paren) stays visible; groups written with `{}` get parentheses only where needed.
// - Function arguments are always parenthesized; positive integer powers of trig
//   functions render as sin²(x), other function powers as (f(x))².

import type { Expr } from "../generated/Expr";
import type { Function as Fn } from "../generated/Function";
import type { Ident } from "../generated/Ident";

export interface LatexCtx {
  /** CSS hex color per user function key, for coloring its name. */
  colors: Map<string, string>;
  /** Subexpression highlights: node id (Sum term ids included) → CSS hex color. */
  nodes?: Map<number, string>;
  /** Keys of named vectors, written with an arrow (\vec{v}). */
  vectors?: Set<string>;
}

/** A definition whose name stands for a vector: a vector value or an arrow. */
export function isVectorName(f: Fn): boolean {
  // A `family` of arrows is a set of vectors, not one.
  return (f.kind === "Value" && f.ty.type === "Vector") || (f.kind === "Arrow" && !f.family);
}

/** Coloring and vector arrows for the given definitions. */
export function latexCtx(fns: Fn[], color: (f: Fn) => string): LatexCtx {
  return { colors: new Map(fns.map((f) => [f.key, color(f)])), vectors: new Set(fns.filter(isVectorName).map((f) => f.key)) };
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

const LINALG = new Set(["matmul", "pow", "inv", "transpose", "det", "tr", "span", "arrow"]);
const BUILTIN_NAMES = new Set([...Object.keys(BUILTIN_TEX), "sqrt", "abs", "floor", "ceil", "log", "mod", "deg", ...LINALG]);

/** Operands that need no parentheses before a superscript or in a product. */
const ATOMS = new Set(["Num", "Name", "List", "Paren", "Index"]);

/** `A_[1, 2]` → a₁₂: a single uppercase letter becomes lowercase (entries of A are aᵢⱼ). */
function elementTex(e: Extract<Expr, { type: "Index" }>, go: (c: Expr, s?: boolean) => string): string {
  const parts = e.indices.map((i) => go(unparen(i), true));
  const simple = e.indices.every((i) => (i.type === "Num" && i.text.length === 1) || (i.type === "Name" && i.name.base.length === 1 && !i.name.sub));
  const sub = simple ? parts.join("") : parts.join(",");
  const b = e.base;
  if (b.type === "Name" && b.name.sub === null && b.name.sup === null && b.name.accent === null) {
    const base = /^[A-Z]$/.test(b.name.base) ? b.name.base.toLowerCase() : b.name.base;
    return `${baseTex(base)}_{${sub}}`;
  }
  return `${paren(go(b))}_{${sub}}`;
}

function matrixTex(e: Extract<Expr, { type: "List" }>, go: (c: Expr, s?: boolean) => string): string {
  const rows = e.items.every((r) => r.type === "List") ? e.items.map((r) => (r.type === "List" ? r.items : [])) : e.items.map((x) => [x]);
  return `\\begin{pmatrix}${rows.map((r) => r.map((x) => go(x, false)).join(" & ")).join(" \\\\ ")}\\end{pmatrix}`;
}

/** `matmul(A, B)` → AB, `transpose(A)` → Aᵀ, `inv(A)` → A⁻¹, `pow(A, n)` → Aⁿ. */
function linalgTex(e: Extract<Expr, { type: "Call" }>, go: (c: Expr, s?: boolean) => string): string | null {
  const a = e.args.map(unparen);
  const atom = (x: Expr) => (ATOMS.has(x.type) || (x.type === "Call" && x.kind === "Function") ? go(x) : paren(go(x)));
  // Superscript forms (Aᵀ, A⁻¹, Aⁿ) and nested products need no parentheses as a factor.
  const factor = (x: Expr) => (x.type === "Call" && x.kind === "Builtin" && ["matmul", "transpose", "inv", "pow"].includes(x.callee.key) ? go(x) : atom(x));
  const sup = (x: Expr, s: string) => `{${atom(x)}}^{${s}}`;
  switch (e.callee.key) {
    case "matmul":
      return a.length === 2 ? `${factor(a[0]!)}${factor(a[1]!)}` : null;
    case "transpose":
      return a[0] ? sup(a[0], "\\mathsf{T}") : null;
    case "inv":
      return a[0] ? sup(a[0], "-1") : null;
    case "pow":
      return a.length === 2 ? sup(a[0]!, go(a[1]!, true)) : null;
    case "det":
      return a[0] ? `\\det${paren(go(a[0]))}` : null;
    case "tr":
      return a[0] ? `\\operatorname{tr}${paren(go(a[0]))}` : null;
    case "span":
      return `\\operatorname{span}${paren(a.map((x) => go(x)).join(", "))}`;
    case "arrow":
      return `\\operatorname{arrow}${paren(a.map((x) => go(x)).join(";\\, "))}`;
  }
  return null;
}

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

/** `vec`: a named vector, written \vec{v} (\overrightarrow over a multi-letter base). */
export function identTex(id: Ident, color?: string, vec = false): string {
  let t = baseTex(id.base);
  if (id.accent) t = `\\${id.accent}{${t}}`;
  else if (vec) t = id.base.length === 1 || /^\\[A-Za-z]+$/.test(id.base) ? `\\vec{${t}}` : `\\overrightarrow{${t}}`;
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
      return identTex(e.name, e.kind === "Function" || e.kind === "Value" ? ctx.colors.get(e.name.key) : undefined, e.kind === "Value" && !!ctx.vectors?.has(e.name.key));
    case "List":
      return matrixTex(e, go);
    case "Index":
      return elementTex(e, go);
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
        const la = LINALG.has(key) ? linalgTex(e, go) : null;
        if (la) return la;
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
    case "Tuple":
      return paren(e.items.map((i) => go(unparen(i))).join(", "));
    case "Split": {
      const rows = e.pieces.map((p) => `${go(p.value)}, & ${go(unparen(p.cond))}`).join(" \\\\ ");
      const cases = `\\begin{cases} ${rows} \\end{cases}`;
      // The subject's name goes on a line under the cases (beside them is too wide for the panel).
      return e.name ? `\\begin{gathered} ${cases} \\\\ \\scriptstyle ${identTex(e.name)} \\,=\\, ${go(unparen(e.subject))} \\end{gathered}` : cases;
    }
    case "Feature": {
      // `extrema(f; -1 < x < 1; max)`: slots as written, `;` between them.
      const slots = [e.args.map((a) => go(unparen(a))).join(", ")];
      if (e.range || e.filter) slots.push(e.range ? go(unparen(e.range)) : "");
      if (e.filter) slots.push(`\\text{${e.filter.replace(/[^A-Za-z]/g, "")}}`);
      return `\\operatorname{${e.callee.key.replace(/[^A-Za-z]/g, "")}}${paren(slots.join(";\\, "))}`;
    }
    case "Calc": {
      const slots = [e.steps.map((a) => go(a)).join(", ")];
      if (e.range || e.kind) slots.push(e.range ? go(unparen(e.range)) : "");
      if (e.kind) slots.push(`\\text{${e.kind.replace(/[^A-Za-z]/g, "")}}`);
      return `\\operatorname{calc}${paren(slots.join(";\\, "))}`;
    }
    default:
      return "\\text{?}";
  }
}

/**
 * `f(x) = body` as shown in the definition list (constants are not shown in the head).
 * Without inputs (vectors, matrices, span, arrow): `v = body`.
 */
export function definitionTex(name: Ident, inputs: Ident[], body: Expr, ctx: LatexCtx): string {
  const n = identTex(name, ctx.colors.get(name.key), !!ctx.vectors?.has(name.key));
  const head = inputs.length ? `${n}${paren(inputs.map((i) => identTex(i)).join(", "))}` : n;
  return `${head} = ${exprTex(body, ctx)}`;
}

/**
 * A definition as listed: `f(x) = …`; parametric / polar with their range on the line
 * under it (`p(t) = (cos t, sin t)` / `0 ≤ t ≤ 2π`); an equation as `C: x² + y² = a²`;
 * `family` as `Ш_{k=0}^{n} { … }`.
 */
export function functionTex(f: Fn, ctx: LatexCtx): string {
  // `family`: Ш with the index range, the body in braces.
  const sha = (body: string) => {
    const e = f.family;
    if (!e) return body;
    const k = identTex(e.index);
    const lo = exprTex(e.lo, ctx, true), hi = exprTex(e.hi, ctx, true);
    const range = !e.lo_strict && !e.hi_strict ? `_{${k}=${lo}}^{${hi}}` : `_{${lo}${e.lo_strict ? "<" : "\\le "}${k}${e.hi_strict ? "<" : "\\le "}${hi}}`;
    return `\\Sha${range} \\left\\{ ${body} \\right\\}`;
  };
  const name = identTex(f.name, ctx.colors.get(f.name.key));
  if (f.kind === "Equation") return `${name}\\colon\\; ${sha(exprTex(f.body, ctx))}`;
  const head = f.inputs.length ? `${name}${paren(f.inputs.map((i) => identTex(i)).join(", "))}` : name;
  const def = f.family ? `${head} = ${sha(exprTex(unparen(f.body), ctx))}` : definitionTex(f.name, f.inputs, f.body, ctx);
  if (!f.range || !f.inputs[0]) return def;
  // The range goes on a line under the definition (beside it is too wide for the panel);
  // a parametric surface has one per parameter, separated by a comma.
  const one = (r: { lo: Expr; hi: Expr }, t: Ident) => `${exprTex(r.lo, ctx)} \\,\\le\\, ${identTex(t)} \\,\\le\\, ${exprTex(r.hi, ctx)}`;
  const ranges = [one(f.range, f.inputs[0]), ...(f.range2 && f.inputs[1] ? [one(f.range2, f.inputs[1])] : [])].join(",\\quad ");
  return `\\begin{gathered} ${def} \\\\ \\scriptstyle ${ranges} \\end{gathered}`;
}

/** Color of the parts that changed from the previous calculation step. */
export const DIFF_COLOR = "#ffcf5c";

/**
 * A calculation (`C: calc(f, g, h)`) as aligned lines: `f(x) = <f>`, `= <g>`, `= <h>`;
 * each line after the first colors what changed from the line before (server diff).
 * `count`: how many steps to show (a scene step reveals the next line).
 */
export function calcTex(f: Fn, fns: Record<string, Fn | undefined>, ctx: LatexCtx, count: number): string {
  const c = f.calc;
  if (!c) return identTex(f.name);
  const steps = c.steps.slice(0, Math.max(1, count)).map((k) => fns[k]);
  if (steps.some((s) => !s)) return identTex(f.name);
  const first = steps[0]!;
  const head = `${identTex(first.name, ctx.colors.get(first.key))}${paren(first.inputs.map((i) => identTex(i)).join(", "))}`;
  // Integral / limit: the relation of the first two lines is not equality.
  const rel = c.kind === "Integral" ? "\\xrightarrow{\\int}" : c.kind === "Limit" ? "\\xrightarrow{\\lim}" : "=";
  const rows = steps.map((s, i) => {
    const after = i > 0 ? (c.diffs[i - 1]?.after ?? []) : [];
    const body = exprTex(s!.body, { ...ctx, nodes: new Map(after.map((id) => [id, DIFF_COLOR])) });
    return `${i === 0 ? head : ""} &${i === 1 ? rel : "="} ${body}`;
  });
  // The calc's own name in front, as for equations (the first line repeats a step's head).
  return `${identTex(f.name)}\\colon\\; \\begin{aligned} ${rows.join(" \\\\ ")} \\end{aligned}`;
}
