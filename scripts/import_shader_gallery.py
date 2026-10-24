#!/usr/bin/env python3
"""Import shader-gallery/shaders (MIT) into Fosfora .pfx + .wgsl."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

BASE = "https://raw.githubusercontent.com/shader-gallery/shaders/main"
FOSFORA = Path(__file__).resolve().parents[1]
EFFECTS = FOSFORA / "assets" / "effects"
SHADERS = FOSFORA / "assets" / "shaders"

# User-selected batch (slug -> optional display name override)
IMPORTS: list[tuple[str, str | None]] = [
    ("dendrite", None),
    ("funnelweb", None),
    ("mycelium", "SG Mycelium"),
    ("gauze", None),
    ("condensation", None),
    ("rainglass", None),
    ("lightshow", None),
    ("stone", None),
    ("undertow", None),
    ("halo", None),
    ("hyperspeed", None),
    ("boids", None),
    ("iris-ringed", None),
    ("damask", None),
    ("quintet", None),
    ("scales", None),
    ("chalk", None),
    ("lode", None),
    ("heartwood", None),
    ("suffuse", None),
    ("frost", "SG Frost"),
    ("ingot", None),
    ("floret", None),
    ("tally", None),
    ("chamber", None),
    ("harlequin", None),
    ("tumbler", None),
    ("gyroid", None),
    ("neovius", None),
    ("opart", None),
    # Palette-themed adds (display names; slugs from shader.gallery)
    ("brick", "Rust"),
    ("quench", "Ember"),
    ("ripple", "SG Tide"),
]

NAME_COLLISION_FILES = {"mycelium", "frost"}


def fetch(url: str) -> str:
    with urllib.request.urlopen(url, timeout=30) as r:
        return r.read().decode("utf-8")


def load_palettes_js() -> dict[str, list[list[float]]]:
    raw = fetch(
        "https://raw.githubusercontent.com/shader-gallery/runtime/main/palettes.js"
    )
    themes: dict[str, list[list[float]]] = {}
    for m in re.finditer(
        r"^\s*([a-z][a-z0-9_]*)\s*:\s*\[\[([^\]]+)\],\s*\[([^\]]+)\],\s*\[([^\]]+)\],\s*\[([^\]]+)\]\],?",
        raw,
        re.M,
    ):
        name = m.group(1)
        cols = []
        for g in m.groups()[1:5]:
            nums = [float(x.strip()) for x in g.split(",")]
            cols.append(nums)
        themes[name] = cols
    return themes


def glsl_type_to_wgsl(t: str) -> str:
    t = t.strip()
    mapping = {
        "float": "f32",
        "int": "i32",
        "bool": "bool",
        "vec2": "vec2f",
        "vec3": "vec3f",
        "vec4": "vec4f",
        "mat2": "mat2x2<f32>",
        "mat3": "mat3x3<f32>",
        "mat4": "mat4x4<f32>",
    }
    return mapping.get(t, t)


def parse_param_list(inner: str) -> list[tuple[str, str, str]]:
    """Return list of (qualifier, type, name). qualifier is '' | 'out' | 'inout'."""
    parts = []
    buf = ""
    depth = 0
    for ch in inner:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(buf.strip())
            buf = ""
        else:
            buf += ch
    if buf.strip():
        parts.append(buf.strip())
    params = []
    for p in parts:
        p = re.sub(r"\s+", " ", p).strip()
        m = re.match(r"^(?:(inout|out)\s+)?(\w+)\s+(\w+)$", p)
        if not m:
            raise ValueError(f"bad param: {p!r}")
        qual, typ, name = m.group(1) or "", m.group(2), m.group(3)
        params.append((qual, typ, name))
    return params


def wgsl_params(params: list[tuple[str, str, str]]) -> str:
    out = []
    for qual, typ, name in params:
        wt = glsl_type_to_wgsl(typ)
        if qual in ("out", "inout"):
            out.append(f"{name}: ptr<function, {wt}>")
        else:
            out.append(f"{name}: {wt}")
    return ", ".join(out)


def pointer_names(params: list[tuple[str, str, str]]) -> list[str]:
    return [name for qual, _, name in params if qual in ("out", "inout")]


def patch_body_pointers(body: str, names: list[str]) -> str:
    if not names:
        return body
    for name in sorted(names, key=len, reverse=True):
        # Skip declarations like `float shade` or `vec2 c`
        body = re.sub(
            rf"\b({'|'.join(['float','f32','int','i32','vec2','vec2f','vec3','vec3f','bool'])})\s+{name}\b",
            rf"\1 {name}",
            body,
        )
        body = re.sub(rf"(?<![\w*]){name}(?![\w])", f"*{name}", body)
    return body


def patch_call_pointers(code: str, fn: str, ptr_positions: list[int]) -> str:
    """Wrap call arguments at 0-based positions with &."""
    pattern = re.compile(rf"(?<!fn )\b{fn}\s*\(")

    def repl(m: re.Match[str]) -> str:
        start = m.end()
        depth = 1
        i = start
        args: list[str] = []
        cur = ""
        while i < len(code) and depth:
            ch = code[i]
            if ch == "(":
                depth += 1
                cur += ch
            elif ch == ")":
                depth -= 1
                if depth == 0:
                    if cur.strip():
                        args.append(cur.strip())
                    break
                cur += ch
            elif ch == "," and depth == 1:
                args.append(cur.strip())
                cur = ""
            else:
                cur += ch
            i += 1
        for pos in ptr_positions:
            if pos < len(args) and not args[pos].startswith("&"):
                args[pos] = "&" + args[pos]
        new_inner = ", ".join(args)
        return f"{fn}({new_inner})"

    # rebuild string by scanning
    out = []
    last = 0
    for m in pattern.finditer(code):
        out.append(code[last : m.start()])
        chunk = repl(m)
        # repl used wrong scope - fix by inline implementation
        start = m.end()
        depth = 1
        i = start
        args: list[str] = []
        cur = ""
        while i < len(code) and depth:
            ch = code[i]
            if ch == "(":
                depth += 1
                cur += ch
            elif ch == ")":
                depth -= 1
                if depth == 0:
                    if cur.strip():
                        args.append(cur.strip())
                    break
                cur += ch
            elif ch == "," and depth == 1:
                args.append(cur.strip())
                cur = ""
            else:
                cur += ch
            i += 1
        for pos in ptr_positions:
            if pos >= len(args) or args[pos].startswith("&"):
                continue
            if args[pos].startswith("*"):
                args[pos] = "&" + args[pos][1:]
            else:
                args[pos] = "&" + args[pos]
        out.append(f"{fn}({', '.join(args)})")
        last = i + 1
    out.append(code[last:])
    return "".join(out)


def convert_functions(code: str) -> str:
    fn_pattern = re.compile(
        r"^void\s+(\w+)\s*\(([^)]*)\)\s*\{",
        re.M,
    )
    matches = list(fn_pattern.finditer(code))
    # process from end to start so indices stay valid
    for m in reversed(matches):
        name = m.group(1)
        if name == "main":
            continue
        params = parse_param_list(m.group(2)) if m.group(2).strip() else []
        ptrs = pointer_names(params)
        ptr_positions = [i for i, (q, _, _) in enumerate(params) if q in ("out", "inout")]

        # find matching closing brace
        start = m.end()
        depth = 1
        i = start
        while i < len(code) and depth:
            if code[i] == "{":
                depth += 1
            elif code[i] == "}":
                depth -= 1
            i += 1
        body = code[start : i - 1]
        body = patch_body_pointers(body, ptrs)
        new_sig = f"fn {name}({wgsl_params(params)}) {{"
        code = code[: m.start()] + new_sig + body + code[i - 1 :]
    return code


def strip_header(glsl: str) -> str:
    lines = []
    for line in glsl.splitlines():
        if line.startswith("// SPDX"):
            continue
        if "precision highp float" in line:
            continue
        if re.match(r"^\s*uniform\s+", line):
            continue
        lines.append(line)
    return "\n".join(lines)


def fix_mat2_constructors(code: str) -> str:
    def repl(m: re.Match[str]) -> str:
        a, b, c, d = m.group(1), m.group(2), m.group(3), m.group(4)
        return f"mat2x2<f32>(vec2f({a}, {b}), vec2f({c}, {d}))"

    return re.sub(
        r"mat2x2<f32>\s*\(\s*([-0-9.]+)\s*,\s*([-0-9.]+)\s*,\s*([-0-9.]+)\s*,\s*([-0-9.]+)\s*\)",
        repl,
        code,
    )


def fix_const_types(code: str) -> str:
    for typ in ("f32", "i32", "vec2f", "vec3f", "vec4f", "mat2x2<f32>"):
        code = re.sub(
            rf"\bconst\s+{re.escape(typ)}\s+(\w+)\s*=",
            rf"const \1: {typ} =",
            code,
        )
    return code


def fix_fn_return_type_declarations(code: str) -> str:
    """GLSL `float foo(` -> `fn foo(...) -> f32`."""
    ret_types = ("f32", "i32", "vec2f", "vec3f", "vec4f", "mat2x2<f32>", "bool")
    for rt in ret_types:
        pattern = rf"^{rt}\s+(\w+)\s*\(([^)]*)\)\s*\{{"

        def repl(m: re.Match[str], rt: str = rt) -> str:
            name, inner = m.group(1), m.group(2)
            params = parse_param_list(inner) if inner.strip() else []
            return f"fn {name}({wgsl_params(params)}) -> {rt} {{"

        code = re.sub(pattern, repl, code, flags=re.M)
    return code


def paren_aware_comma_chunks(rhs: str) -> list[str]:
    """Split `a = expr, b = expr` on top-level commas only."""
    chunks: list[str] = []
    buf = ""
    depth = 0
    i = 0
    while i < len(rhs):
        ch = rhs[i]
        if ch == "(":
            depth += 1
            buf += ch
        elif ch == ")":
            depth -= 1
            buf += ch
        elif ch == "," and depth == 0:
            rest = rhs[i + 1 :].lstrip()
            if re.match(r"\w+\s*=", rest):
                chunks.append(buf.strip())
                buf = ""
                i += 1
                continue
            buf += ch
        else:
            buf += ch
        i += 1
    if buf.strip():
        chunks.append(buf.strip())
    return chunks


def split_comma_decls(code: str) -> str:
    """`f32 a = 1.0, b = 2.0;` -> separate let lines."""

    def split_line(line: str) -> str:
        m_brace = re.match(r"^(\s*)\{\s*let\s+(\w+)\s*=\s*(.+);\s*$", line)
        if m_brace and "," in m_brace.group(3):
            ind, first, rest = m_brace.group(1), m_brace.group(2), m_brace.group(3)
            if re.search(r",\s*\w+\s*=", rest):
                chunks = paren_aware_comma_chunks(f"{first} = {rest}")
                inner = "\n".join(f"{ind}  let {c};" for c in chunks)
                return f"{ind}{{\n{inner}"
        line = re.sub(
            r"^(\s*)mat2x2<f32>\s+(\w+)\s*=\s*",
            r"\1let \2 = ",
            line,
        )
        m = re.match(
            r"^(\s*)(f32|vec2f|vec3f|vec4f|i32|bool)\s+(\w+)\s*=\s*(.+);\s*$",
            line,
        )
        if not m:
            m = re.match(r"^(\s*)let\s+(\w+)\s*=\s*(.+);\s*$", line)
            if m:
                indent, first, rest = m.group(1), m.group(2), m.group(3)
                if "," in rest and re.search(r",\s*\w+\s*=", rest):
                    chunks = paren_aware_comma_chunks(f"{first} = {rest}")
                    return "\n".join(f"{indent}let {c};" for c in chunks)
            m = re.match(r"^(\s*)let\s+(\w+)\s*=\s*(.+);\s*$", line)
            if not m:
                return line
            indent, first, rest = m.group(1), m.group(2), m.group(3)
            if "," not in rest or not re.search(r",\s*\w+\s*=", rest):
                return line
            chunks = paren_aware_comma_chunks(f"{first} = {rest}")
            return "\n".join(f"{indent}let {c};" for c in chunks)

        indent, _typ, first, rest = m.group(1), m.group(2), m.group(3), m.group(4)
        if "," not in rest or not re.search(r",\s*\w+\s*=", rest):
            return f"{indent}let {first} = {rest};"
        chunks = paren_aware_comma_chunks(f"{first} = {rest}")
        return "\n".join(f"{indent}let {c};" for c in chunks)

    return "\n".join(split_line(ln) for ln in code.splitlines())


def locals_to_let(code: str) -> str:
    code = re.sub(
        r"^(\s*)(f32|vec2f|vec3f|vec4f|i32|bool)\s+(\w+)\s*=",
        r"\1let \3 =",
        code,
        flags=re.M,
    )
    code = re.sub(r"\{\s*f32\s+(\w+)\s*=", r"{ let \1 =", code)
    code = re.sub(r"\{\s*vec2f\s+(\w+)\s*=", r"{ let \1 =", code)
    code = re.sub(r"\{\s*vec3f\s+(\w+)\s*=", r"{ let \1 =", code)
    code = re.sub(
        r"\{\s*(f32|vec2f|vec3f|vec4f|i32|bool)\s+(\w+)\s*=",
        r"{ let \2 =",
        code,
    )
    return code


def split_wgsl_params(inner: str) -> list[str]:
    """Split WGSL param list on commas not inside `<...>`."""
    parts: list[str] = []
    buf = ""
    depth = 0
    for ch in inner:
        if ch == "<":
            depth += 1
            buf += ch
        elif ch == ">":
            depth -= 1
            buf += ch
        elif ch == "," and depth == 0:
            if buf.strip():
                parts.append(buf.strip())
            buf = ""
        else:
            buf += ch
    if buf.strip():
        parts.append(buf.strip())
    return parts


def parse_wgsl_param_names(inner: str) -> list[tuple[str, str]]:
    """Return (name, type) for simple WGSL params (no ptr)."""
    out: list[tuple[str, str]] = []
    for chunk in split_wgsl_params(inner):
        if not chunk or "ptr<" in chunk:
            continue
        m = re.match(r"(\w+)\s*:\s*([\w<>]+)", chunk)
        if m:
            out.append((m.group(1), m.group(2)))
    return out


def fix_mutated_bindings(code: str) -> str:
    """`let` + later mutation -> `var`; reassigned fn params get a local `var`."""

    fn_re = re.compile(
        r"fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*[\w<>,]+)?\s*\{",
        re.M,
    )
    for m in list(fn_re.finditer(code))[::-1]:
        if m.group(1) == "fs_main":
            continue
        sig_start, body_start = m.start(), m.end()
        depth = 1
        i = body_start
        while i < len(code) and depth:
            if code[i] == "{":
                depth += 1
            elif code[i] == "}":
                depth -= 1
            i += 1
        body = code[body_start : i - 1]
        sig = code[sig_start:body_start]
        prefix = ""
        for name, wt in parse_wgsl_param_names(m.group(2)):
            if re.search(rf"(?<![\w*]){name}\s*(?:\+=|-=|\*=|=(?!=))", body):
                sig = sig.replace(f"{name}: {wt}", f"{name}_in: {wt}", 1)
                prefix += f"  var {name} = {name}_in;\n"
        for name in set(re.findall(r"\blet\s+(\w+)\s*=", body)):
            if _local_is_mutated(body, name):
                body = re.sub(rf"\blet\s+{name}\s*=", f"var {name} =", body)
        if prefix:
            body = prefix + body
        sig = re.sub(r"\{\s*", "{\n", sig, count=1)
        code = code[:sig_start] + sig + body + code[i - 1 :]
    return code


def fix_control_flow_braces(code: str) -> str:
    code = re.sub(
        r"if\s*\(([^)]+)\)\s+(\w+(?:\.\w+)?)\s*=\s*([^;]+);",
        r"if (\1) { \2 = \3; }",
        code,
    )
    code = re.sub(
        r"if\s*\(([^)]+)\)\s+return\s+([^;]+);",
        r"if (\1) { return \2; }",
        code,
    )
    code = re.sub(
        r"if\s*\(([^)]+)\)\s+return\s*;",
        r"if (\1) { return; }",
        code,
    )
    code = re.sub(
        r"if\s*\(([^)]+)\)\s+continue\s*;",
        r"if (\1) { continue; }",
        code,
    )
    code = re.sub(
        r"if\s*\(([^)]+)\)\s+break\s*;",
        r"if (\1) { break; }",
        code,
    )
    return code


def fix_global_comma_vecs(code: str) -> str:
    def repl(m: re.Match[str]) -> str:
        names = [n.strip() for n in m.group(1).split(",")]
        return "\n".join(f"var<private> {n}: vec3f;" for n in names)

    code = re.sub(r"^vec3f\s+((?:\w+\s*,\s*)+\w+)\s*;\s*$", repl, code, flags=re.M)
    code = re.sub(
        r"^f32\s+((?:\w+\s*,\s*)+\w+)\s*;\s*$",
        lambda m: "\n".join(
            f"var<private> {n.strip()}: f32;" for n in m.group(1).split(",")
        ),
        code,
        flags=re.M,
    )
    return code


def fix_ternary_exprs(code: str) -> str:
    # Leave ternaries for manual follow-up; GLSL `?:` is not valid WGSL.
    # Convert only simple scalar ternaries on one line without nesting.
    pat = re.compile(
        r"(\b[\w.]+\s*[<>=!]+\s*[\w.]+\s*)\?\s*([\w.]+)\s*:\s*([\w.]+)"
    )
    for _ in range(16):
        new = pat.sub(r"select(\3, \2, \1)", code)
        if new == code:
            break
        code = new
    return code


def fix_for_f32_loops(code: str) -> str:
    return re.sub(r"for\s*\(\s*f32\s+(\w+)\s*=", r"for (var \1: f32 =", code)


def fix_else_return(code: str) -> str:
    code = re.sub(r"\belse\s+return\s*;", "else { return; }", code)
    return code


def fix_uninitialized_decls(code: str) -> str:
    """`vec2f ca, cb;` -> `var ca: vec2f; var cb: vec2f;`"""

    def repl(m: re.Match[str]) -> str:
        typ = m.group(1)
        names = [n.strip() for n in m.group(2).split(",")]
        return "\n".join(f"var {n}: {typ};" for n in names)

    code = re.sub(
        r"^\s*(vec2f|vec3f|vec4f|f32|bool)\s+((?:\w+\s*,\s*)+\w+)\s*;\s*$",
        repl,
        code,
        flags=re.M,
    )
    code = re.sub(
        r"^\s*(vec2f|vec3f|vec4f|f32|bool)\s+(\w+)\s*;\s*$",
        lambda m: f"var {m.group(2)}: {m.group(1)};",
        code,
        flags=re.M,
    )
    code = re.sub(
        r";\s*(f32|vec2f|vec3f)\s+(\w+)\s*=",
        r";\n  var \2 =",
        code,
    )
    return code


def fix_glsl_mixed_decl_lines(code: str) -> str:
    """`vec2f c; f32 R;` and `bool ina = x, inb = y;` on one line."""

    def split_line(line: str) -> str:
        stripped = line.strip()
        m = re.match(
            r"^bool\s+(\w+)\s*=\s*(.+),\s*(\w+)\s*=\s*(.+);\s*$",
            stripped,
        )
        if m:
            indent = line[: len(line) - len(line.lstrip())]
            a, ea, b, eb = m.group(1), m.group(2), m.group(3), m.group(4)
            return f"{indent}let {a} = {ea};\n{indent}let {b} = {eb};"
        parts = [p.strip() for p in stripped.split(";") if p.strip()]
        if len(parts) <= 1:
            # `vec2f ca, cb;` on one segment
            m2 = re.match(
                r"^(vec2f|vec3f|vec4f|f32|bool)\s+((?:\w+\s*,\s*)+\w+)\s*$",
                stripped.rstrip(";"),
            )
            if m2:
                indent = line[: len(line) - len(line.lstrip())]
                typ = m2.group(1)
                names = [n.strip() for n in m2.group(2).split(",")]
                return "\n".join(f"{indent}var {n}: {typ};" for n in names)
            return line
        if not all(re.match(r"^(vec2f|vec3f|vec4f|f32|bool)\s+\w+", p) for p in parts):
            return line
        indent = line[: len(line) - len(line.lstrip())]
        out: list[str] = []
        for part in parts:
            pm = re.match(
                r"^(vec2f|vec3f|vec4f|f32|bool)\s+((?:\w+\s*,\s*)+\w+|\w+)\s*$",
                part,
            )
            if pm:
                typ = pm.group(1)
                for n in [x.strip() for x in pm.group(2).split(",")]:
                    out.append(f"{indent}var {n}: {typ};")
        return "\n".join(out) if out else line

    return "\n".join(split_line(ln) for ln in code.splitlines())


def fix_out_fn_calls(code: str, decls: str = "") -> str:
    """Add & at call sites for functions with ptr params (heuristic)."""
    fn_re = re.compile(
        r"fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*[\w<>,]+)?\s*\{",
        re.M,
    )
    ptr_fns: dict[str, list[int]] = {}
    for m in fn_re.finditer(decls + "\n" + code if decls else code):
        params = m.group(2)
        if "ptr<function" not in params:
            continue
        positions = []
        for i, chunk in enumerate(split_wgsl_params(params)):
            if "ptr<function" in chunk:
                positions.append(i)
        if positions:
            ptr_fns[m.group(1)] = positions
    for fn, positions in ptr_fns.items():
        code = patch_call_pointers(code, fn, positions)
    return code


def _local_is_mutated(body: str, name: str) -> bool:
    tail = body.split(f"let {name} =", 1)[1] if f"let {name} =" in body else body
    if re.search(rf"\b{name}\s*(?:\+=|-=|\*=|=(?!=))", tail):
        return True
    if re.search(rf"\b{name}\.[xyzwxy]+\s*(?:\+=|-=|\*=|=(?!=))", tail):
        return True
    return False


def fix_mutated_locals_block(body: str) -> str:
    for name in set(re.findall(r"\blet\s+(\w+)\s*=", body)):
        if _local_is_mutated(body, name):
            body = re.sub(rf"\blet\s+{name}\s*=", f"var {name} =", body)
    return body


def _ternary_colon_for(code: str, q_idx: int) -> int:
    depth = 0
    i = q_idx + 1
    while i < len(code):
        ch = code[i]
        if ch == "?":
            depth += 1
        elif ch == ":":
            if depth == 0:
                return i
            depth -= 1
        i += 1
    return -1


def _ternary_cond_start(code: str, q_idx: int) -> int:
    i = q_idx - 1
    while i >= 0 and code[i].isspace():
        i -= 1
    if i >= 0 and code[i] == ")":
        depth = 1
        i -= 1
        while i >= 0 and depth:
            if code[i] == ")":
                depth += 1
            elif code[i] == "(":
                depth -= 1
            i -= 1
        return i + 1
    depth = 0
    while i >= 0:
        ch = code[i]
        if ch == ")":
            depth += 1
        elif ch == "(":
            depth -= 1
            if depth < 0:
                return i + 1
        elif depth == 0 and ch in ",;{(:?":
            return i + 1
        elif depth == 0 and ch == "=":
            if (i + 1 < len(code) and code[i + 1] == "=") or (
                i > 0 and code[i - 1] in "<>!="
            ):
                pass
            else:
                return i + 1
        i -= 1
    return 0


def _ternary_false_end(code: str, colon: int) -> int:
    t_depth = 0
    p_depth = 0
    i = colon + 1
    while i < len(code):
        ch = code[i]
        if ch == "(":
            p_depth += 1
        elif ch == ")":
            if p_depth == 0 and t_depth == 0:
                return i
            p_depth -= 1
        elif ch == "?" and p_depth == 0:
            t_depth += 1
        elif ch == ":" and p_depth == 0:
            if t_depth == 0:
                break
            t_depth -= 1
        elif p_depth == 0 and t_depth == 0 and ch in ",;":
            return i
        i += 1
    return i


def convert_glsl_ternaries(code: str) -> str:
    """Innermost GLSL `a ? b : c` -> WGSL `select(c, b, a)`."""
    for _ in range(512):
        q = code.rfind("?")
        if q < 0:
            break
        line_start = code.rfind("\n", 0, q) + 1
        if "//" in code[line_start:q]:
            code = code[:q] + "\x00" + code[q + 1 :]
            continue
        colon = _ternary_colon_for(code, q)
        if colon < 0:
            break
        false_end = _ternary_false_end(code, colon)
        start = _ternary_cond_start(code, q)
        cond = code[start:q].strip()
        true_part = code[q + 1 : colon].strip()
        false_part = code[colon + 1 : false_end].strip()
        repl = f"select({false_part}, {true_part}, {cond})"
        code = code[:start] + repl + code[false_end:]
    return code.replace("\x00", "?")


def fix_swizzle_assignments(code: str) -> str:
    code = re.sub(
        r"\b(\w+)\.z\s*\*=\s*([^;]+);",
        r"\1 = vec3f(\1.x, \1.y, \1.z * (\2));",
        code,
    )
    code = re.sub(
        r"\b(\w+)\.xy\s*\+=\s*([^;]+);",
        r"let _swA = \2; \1 = vec3f(\1.x + _swA.x, \1.y + _swA.y, \1.z);",
        code,
    )
    code = re.sub(
        r"\b(\w+)\.xz\s*\+=\s*([^;]+);",
        r"let _swA = \2; \1 = vec3f(\1.x + _swA.x, \1.y, \1.z + _swA.y);",
        code,
    )

    def _is_vec3(var: str) -> bool:
        if re.search(rf"\b{var}\.z\b", code):
            return True
        return bool(
            re.search(rf"\b(?:var|let)\s+{var}\s*=[^;]*vec3f", code)
        )

    def repl_xy(m: re.Match[str]) -> str:
        var, expr = m.group(1), m.group(2).strip()
        if _is_vec3(var):
            return f"let _swXY = {expr}; {var} = vec3f(_swXY.x, _swXY.y, {var}.z);"
        return f"let _swXY = {expr}; {var} = vec2f(_swXY.x, _swXY.y);"

    return re.sub(r"\b(\w+)\.xy\s*=\s*([^;]+);", repl_xy, code)


def fix_uniform_u_shadow(code: str) -> str:
    """Local `u` must not shadow the uniform struct when `u.time` is used."""
    fn_re = re.compile(
        r"fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*[\w<>,]+)?\s*\{",
        re.M,
    )
    for m in list(fn_re.finditer(code))[::-1]:
        if m.group(1) == "fs_main":
            continue
        body_start = m.end()
        depth = 1
        i = body_start
        while i < len(code) and depth:
            if code[i] == "{":
                depth += 1
            elif code[i] == "}":
                depth -= 1
            i += 1
        body = code[body_start : i - 1]
        if not re.search(r"\b(?:let|var)\s+u\s*=", body):
            continue
        if not re.search(r"\bu\.(?:time|resolution)\b", body):
            continue
        body = re.sub(r"\blet\s+u\s*=", "let uRim =", body)
        body = re.sub(r"\bvar\s+u\s*=", "var uRim =", body)
        body = re.sub(r"(?<![.\w])u(?![.\w.])", "uRim", body)
        code = code[:body_start] + body + code[i - 1 :]
    return code


def fix_ptr_param_assignments(code: str) -> str:
    """`*param` reads in bodies; assignments to ptr params need `*name =`."""
    fn_re = re.compile(
        r"fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*[\w<>,]+)?\s*\{",
        re.M,
    )
    for m in list(fn_re.finditer(code))[::-1]:
        ptrs = [
            chunk.split(":")[0].strip()
            for chunk in split_wgsl_params(m.group(2))
            if "ptr<function" in chunk
        ]
        if not ptrs:
            continue
        body_start = m.end()
        depth = 1
        i = body_start
        while i < len(code) and depth:
            if code[i] == "{":
                depth += 1
            elif code[i] == "}":
                depth -= 1
            i += 1
        body = code[body_start : i - 1]
        for name in ptrs:
            body = re.sub(
                rf"(?<!\*)\b{name}\s*=",
                f"*{name} =",
                body,
            )
            body = re.sub(
                rf"(?<!\*)\b{name}\s*\+=",
                f"*{name} +=",
                body,
            )
        code = code[:body_start] + body + code[i - 1 :]
    return code


def fix_ptr_arg_locals(code: str) -> str:
    """`let x = ...; f(..., &x)` requires `var x`."""
    for name in set(re.findall(r"&(\w+)", code)):
        code = re.sub(rf"\blet\s+{name}\s*=", f"var {name} =", code, count=1)
    return code


def fix_star_call_args(code: str) -> str:
    """Call sites written as `*param` should pass `&param` for ptr formals."""
    return re.sub(
        r"(,\s*)\*(\w+)(?=\s*[,)])",
        r"\1&\2",
        code,
    )


def fix_inline_cos_sin_lets(code: str) -> str:
    code = re.sub(
        r"\{\s*let\s+(\w+)\s*=\s*cos\(([^)]+)\),\s*(\w+)\s*=\s*sin\(\2\);",
        r"{ let \1 = cos(\2);\n  let \3 = sin(\2);",
        code,
    )
    code = re.sub(
        r"\b(?:var|let)\s+c\s*=\s*cos\(([^)]+)\),\s*s\s*=\s*sin\(\1\);",
        r"var c = cos(\1);\n  var s = sin(\1);",
        code,
    )
    return code


def fix_vec3_clamp_bounds(code: str) -> str:
    subs = [
        (
            r"clamp\(waterDye \* \(0\.24 \+ 0\.16 \* mottle\) \+ 0\.02, 0\.01, 1\.0\)",
            "clamp(waterDye * (0.24 + 0.16 * mottle) + 0.02, vec3f(0.01), vec3f(1.0))",
        ),
        (
            r"clamp\(mix\(vec3f\(lum\), dye, 1\.05\), 0\.03, 1\.0\)",
            "clamp(mix(vec3f(lum), dye, 1.05), vec3f(0.03), vec3f(1.0))",
        ),
        (
            r"clamp\(pastel \* 1\.05, 0\.0, 1\.0\)",
            "clamp(pastel * 1.05, vec3f(0.0), vec3f(1.0))",
        ),
        (
            r"clamp\((\w+Tint), 0\.0, 1\.0\)",
            r"clamp(\1, vec3f(0.0), vec3f(1.0))",
        ),
    ]
    for pat, rep in subs:
        code = re.sub(pat, rep, code)
    return code


def fix_nested_ptr_ampersands(code: str) -> str:
    """Do not take `&` of a function-scope ptr parameter when passing it onward."""
    fn_re = re.compile(
        r"fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*[\w<>,]+)?\s*\{",
        re.M,
    )
    for m in list(fn_re.finditer(code))[::-1]:
        ptr_names = {
            chunk.split(":")[0].strip()
            for chunk in split_wgsl_params(m.group(2))
            if "ptr<function" in chunk
        }
        if not ptr_names:
            continue
        body_start = m.end()
        depth = 1
        i = body_start
        while i < len(code) and depth:
            if code[i] == "{":
                depth += 1
            elif code[i] == "}":
                depth -= 1
            i += 1
        body = code[body_start : i - 1]
        for name in ptr_names:
            body = re.sub(rf"&{name}\b(?=\s*[,)])", name, body)
        code = code[:body_start] + body + code[i - 1 :]
    return code


def fix_scale_at_calls(code: str) -> str:
    return re.sub(
        r"scaleAt\(([^,]+,\s*[^,]+,\s*&\w+),\s*(\w+),\s*(&\w+)\)",
        r"scaleAt(\1, &\2, \3)",
        code,
    )


def fix_sd_box_max_vec2(code: str) -> str:
    """SDF idiom `length(max(d, 0.0))` needs vec2 second arg when d is vec2."""
    code = re.sub(
        r"length\s*\(\s*max\s*\(\s*d\s*,\s*0\.0\s*\)\s*\)",
        "length(max(d, vec2f(0.0)))",
        code,
    )
    code = re.sub(
        r"length\s*\(\s*max\s*\(\s*dd\s*,\s*0\.0\s*\)\s*\)",
        "length(max(dd, vec2f(0.0)))",
        code,
    )
    return code


def strip_redundant_main_decls(body: str) -> str:
    body = re.sub(r"^\s*vec2f\s+fc\s*=\s*fc\s*;\s*\n", "", body, flags=re.M)
    body = re.sub(
        r"^\s*vec2f\s+res\s*=\s*u\.resolution\s*;\s*\n",
        "",
        body,
        flags=re.M,
    )
    body = re.sub(
        r"^\s*let\s+res\s*=\s*u\.resolution\s*;\s*\n",
        "",
        body,
        flags=re.M,
    )
    return body


def convert_types(code: str) -> str:
    # const declarations (float/int before vec* rewrite)
    code = re.sub(
        r"\bconst\s+float\s+(\w+)\s*=",
        r"const \1: f32 =",
        code,
    )
    code = re.sub(
        r"\bconst\s+int\s+(\w+)\s*=",
        r"const \1: i32 =",
        code,
    )
    # globals at line start
    code = re.sub(
        r"^float\s+(\w+)\s*=\s*([^;]+);",
        r"var<private> \1: f32 = \2;",
        code,
        flags=re.M,
    )
    code = re.sub(
        r"^int\s+(\w+)\s*=\s*([^;]+);",
        r"var<private> \1: i32 = \2;",
        code,
        flags=re.M,
    )
    # for loops
    code = re.sub(
        r"for\s*\(\s*int\s+(\w+)\s*=\s*([^;]+);\s*([^;]+);\s*([^)]+)\)",
        r"for (var \1: i32 = \2; \3; \4)",
        code,
    )
    code = re.sub(
        r"for\s*\(\s*f32\s+(\w+)\s*=\s*([^;]+);\s*([^;]+);\s*([^)]+)\)",
        r"for (var \1: f32 = \2; \3; \4)",
        code,
    )
    code = re.sub(
        r"for\s*\(\s*float\s+(\w+)\s*=\s*([^;]+);\s*([^;]+);\s*([^)]+)\)",
        r"for (var \1: f32 = \2; \3; \4)",
        code,
    )
    replacements = [
        (r"\bmat2\b", "mat2x2<f32>"),
        (r"\bmat3\b", "mat3x3<f32>"),
        (r"\bmat4\b", "mat4x4<f32>"),
        (r"\bvec2\b", "vec2f"),
        (r"\bvec3\b", "vec3f"),
        (r"\bvec4\b", "vec4f"),
        (r"\bfloat\b", "f32"),
        (r"\bint\b", "i32"),
        (r"\bmod\s*\(", "sg_mod("),
        (r"\batan\s*\(", "atan2("),
        (r"\blet\s+ref\s*=", "let refl ="),
        (r"\bref\b(?=\s*[,;=)])", "refl"),
        (r"\benv\(ref\)", "env(refl)"),
        (r"\benv\(refl\)", "env(refl)"),
        (r"\bdot\(ref,", "dot(refl,"),
        (r"\breflect\(ref,", "reflect(refl,"),
        (r"(\w)\+\+", r"\1 += 1"),
        (r"(\w)--", r"\1 -= 1"),
        (r"6\.2831\b", "6.283185307"),
        (r"6\.28318\b", "6.283185307"),
    ]
    for pat, rep in replacements:
        code = re.sub(pat, rep, code)
    code = fix_const_types(code)
    code = fix_mat2_constructors(code)
    code = fix_fn_return_type_declarations(code)
    code = re.sub(r"\)\s*->\s*void\s*\{", ") {", code)
    return code


def extract_main(code: str) -> str:
    m = re.search(r"void\s+main\s*\(\s*\)\s*\{", code)
    if not m:
        raise ValueError("no void main()")
    start = m.end()
    depth = 1
    i = start
    while i < len(code) and depth:
        if code[i] == "{":
            depth += 1
        elif code[i] == "}":
            depth -= 1
        i += 1
    main_body = code[: m.start()] + code[start : i - 1]
    return main_body.strip()


def apply_uniform_replacements(code: str, param_map: dict[str, int]) -> str:
    code = re.sub(r"\bu_time\b", "u.time", code)
    code = re.sub(r"\bu_resolution\b", "u.resolution", code)
    code = re.sub(r"\bu_mouse\b", "vec2f(0.0)", code)
    code = re.sub(r"\bu_pixelRatio\b", "1.0", code)
    for uname, idx in param_map.items():
        code = re.sub(rf"\b{uname}\b", f"param({idx}u)", code)
    code = re.sub(r"\bu_palette\s*\[\s*0\s*\]", "SG_C0", code)
    code = re.sub(r"\bu_palette\s*\[\s*1\s*\]", "SG_C1", code)
    code = re.sub(r"\bu_palette\s*\[\s*2\s*\]", "SG_C2", code)
    code = re.sub(r"\bu_palette\s*\[\s*3\s*\]", "SG_C3", code)
    # remove palette fallback block
    code = re.sub(
        r"if\s*\(\s*dot\s*\(\s*c0\s*,\s*c0\s*\).*?\}\s*",
        "",
        code,
        flags=re.S,
    )
    # multi-declare vec3 c0 = SG_C0, ...
    code = re.sub(
        r"vec3f\s+c0\s*=\s*SG_C0\s*,\s*c1\s*=\s*SG_C1\s*,\s*c2\s*=\s*SG_C2\s*,\s*c3\s*=\s*SG_C3\s*;",
        "let c0 = SG_C0; let c1 = SG_C1; let c2 = SG_C2; let c3 = SG_C3;",
        code,
    )
    return code


def wrap_fs_main(body: str) -> str:
    body = re.sub(r"gl_FragColor\s*=\s*", "return ", body)
    body = re.sub(r";\s*$", ";", body.strip())
    if not body.rstrip().endswith(";") and "return " not in body.splitlines()[-1]:
        pass
    return f"""
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    {body}
}}
"""


def convert_shader(glsl: str, meta: dict, palette: list[list[float]]) -> tuple[str, list[dict]]:
    param_defs = meta.get("params") or []
    param_map = {p["uniform"]: i for i, p in enumerate(param_defs)}

    code = strip_header(glsl)
    code = apply_uniform_replacements(code, param_map)
    main_body = extract_main(code)
    prefix = code[: code.find(main_body)].strip() if main_body in code else ""
    # prefix is everything before main body inside extract_main - fix extraction
    code_wo_main = extract_main(strip_header(glsl))
    # Rebuild: convert everything except main
    full = strip_header(glsl)
    full = apply_uniform_replacements(full, param_map)
    m = re.search(r"void\s+main\s*\(\s*\)\s*\{", full)
    if not m:
        raise ValueError("main not found")
    pre = full[: m.start()]
    start = m.end()
    depth = 1
    i = start
    while i < len(full) and depth:
        if full[i] == "{":
            depth += 1
        elif full[i] == "}":
            depth -= 1
        i += 1
    main_body = full[start : i - 1]

    pre = convert_functions(pre)
    pre = convert_types(pre)
    main_body = convert_types(main_body)
    main_body = re.sub(r"gl_FragCoord\.xy", "fc", main_body)
    main_body = re.sub(r"gl_FragCoord", "fc", main_body)
    main_body = strip_redundant_main_decls(main_body)
    pre = re.sub(r";\s*(return\b)", r";\n  \1", pre)
    main_body = re.sub(r";\s*(return\b)", r";\n  \1", main_body)
    main_body = split_comma_decls(main_body)
    main_body = locals_to_let(main_body)
    main_body = split_comma_decls(main_body)
    pre = split_comma_decls(pre)
    pre = locals_to_let(pre)
    pre = split_comma_decls(pre)
    pre = fix_mutated_bindings(pre)
    main_body = fix_mutated_bindings(main_body)

    c0, c1, c2, c3 = palette
    header = f"""// {meta.get('name', 'Shader')} — shader.gallery ({meta.get('slug', '')})
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: {meta.get('defaultPalette', 'midnight')}

const SG_C0: vec3f = vec3f({c0[0]:.6f}, {c0[1]:.6f}, {c0[2]:.6f});
const SG_C1: vec3f = vec3f({c1[0]:.6f}, {c1[1]:.6f}, {c1[2]:.6f});
const SG_C2: vec3f = vec3f({c2[0]:.6f}, {c2[1]:.6f}, {c2[2]:.6f});
const SG_C3: vec3f = vec3f({c3[0]:.6f}, {c3[1]:.6f}, {c3[2]:.6f});

fn sg_mod(x: f32, y: f32) -> f32 {{
    return x - y * floor(x / y);
}}

"""
    pre = fix_global_comma_vecs(pre)
    pre = fix_control_flow_braces(pre)
    main_body = fix_control_flow_braces(main_body)
    pre = fix_for_f32_loops(pre)
    pre = fix_else_return(pre)
    pre = fix_uninitialized_decls(pre)
    pre = fix_glsl_mixed_decl_lines(pre)
    main_body = fix_uninitialized_decls(main_body)
    main_body = fix_glsl_mixed_decl_lines(main_body)
    pre = convert_glsl_ternaries(pre)
    main_body = convert_glsl_ternaries(main_body)
    pre = fix_swizzle_assignments(pre)
    main_body = fix_swizzle_assignments(main_body)
    pre = fix_sd_box_max_vec2(pre)
    main_body = fix_sd_box_max_vec2(main_body)
    pre = fix_uniform_u_shadow(pre)
    main_body = fix_uniform_u_shadow(main_body)
    pre = fix_ptr_param_assignments(pre)
    for _ in range(4):
        pre = fix_out_fn_calls(pre, pre)
        main_body = fix_out_fn_calls(main_body, pre)
    main_body = fix_mutated_locals_block(main_body)
    main_body = fix_ptr_param_assignments(main_body)
    for _ in range(4):
        pre = fix_out_fn_calls(pre, pre)
        main_body = fix_out_fn_calls(main_body, pre)
    pre = fix_mutated_locals_block(pre)
    pre = fix_ptr_arg_locals(pre)
    main_body = fix_ptr_arg_locals(main_body)
    for _ in range(2):
        pre = fix_out_fn_calls(pre, pre)
        main_body = fix_out_fn_calls(main_body, pre)
        pre = fix_ptr_arg_locals(pre)
        main_body = fix_ptr_arg_locals(main_body)
    pre = fix_star_call_args(pre)
    main_body = fix_star_call_args(main_body)
    pre = fix_nested_ptr_ampersands(pre)
    main_body = fix_nested_ptr_ampersands(main_body)
    pre = fix_scale_at_calls(pre)
    main_body = fix_scale_at_calls(main_body)
    pre = fix_inline_cos_sin_lets(pre)
    main_body = fix_inline_cos_sin_lets(main_body)
    pre = fix_vec3_clamp_bounds(pre)
    main_body = fix_vec3_clamp_bounds(main_body)
    pre = re.sub(
        r"return length\(ap - ab \* u\);",
        "return length(ap - ab * (*u));",
        pre,
    )
    main_body = re.sub(
        r"return length\(ap - ab \* u\);",
        "return length(ap - ab * (*u));",
        main_body,
    )
    if "var<private> c0:" in pre:
        main_body = re.sub(r"^\s*let c[0-3] = SG_C[0-3];\s*\n", "", main_body, flags=re.M)
    wgsl = header + pre + wrap_fs_main(main_body)
    inputs = []
    for p in param_defs:
        inputs.append(
            {
                "type": "Float",
                "name": p["uniform"][2:] if p["uniform"].startswith("u_") else p["uniform"],
                "default": p.get("default", 0.5),
                "min": p.get("min", 0.0),
                "max": p.get("max", 1.0),
            }
        )
    return wgsl, inputs


def effect_basename(slug: str) -> str:
    if slug in NAME_COLLISION_FILES:
        return f"sg_{slug.replace('-', '_')}"
    return slug.replace("-", "_")


def write_pfx(path: Path, name: str, meta: dict, shader_file: str, inputs: list[dict]) -> None:
    desc = meta.get("description", "")
    desc = f"{desc} Imported from shader.gallery (MIT)."
    obj = {
        "name": name,
        "author": meta.get("author", "E. T. Carter"),
        "description": desc,
        "shader": "",
        "passes": [{"name": "main", "shader": shader_file, "feedback": False}],
        "inputs": inputs,
        "postprocess": {
            "enabled": True,
            "bloom_enabled": False,
            "ca_enabled": False,
            "grain_enabled": False,
            "vignette_enabled": False,
            "tonemap": "linear",
        },
    }
    path.write_text(json.dumps(obj, indent=4) + "\n")


def validate_all() -> list[str]:
    cmd = [
        "cargo",
        "test",
        "-p",
        "fosfora-app",
        "shipped_pfx_passes_validate_at_baseline_capabilities",
        "--",
        "--nocapture",
    ]
    r = subprocess.run(cmd, cwd=FOSFORA, capture_output=True, text=True)
    if r.returncode == 0:
        return []
    errs = []
    for line in (r.stdout + r.stderr).splitlines():
        if " / " in line and ".wgsl" in line:
            errs.append(line.strip())
    return errs


def main() -> int:
    themes = load_palettes_js()
    failures: list[str] = []

    for slug, name_override in IMPORTS:
        try:
            meta = json.loads(fetch(f"{BASE}/{slug}/meta.json"))
            glsl = fetch(f"{BASE}/{slug}/shader.frag")
            theme = meta.get("defaultPalette", "midnight")
            palette = themes.get(theme) or themes["midnight"]
            wgsl, inputs = convert_shader(glsl, meta, palette)
            base = effect_basename(slug)
            wgsl_name = f"{base}.wgsl"
            pfx_name = f"{base}.pfx"
            display = name_override or meta.get("name", slug)
            (SHADERS / wgsl_name).write_text(wgsl)
            write_pfx(EFFECTS / pfx_name, display, meta, wgsl_name, inputs)
            print(f"OK {slug} -> {pfx_name}")
        except Exception as e:
            failures.append(f"{slug}: {e}")
            print(f"FAIL {slug}: {e}", file=sys.stderr)

    if failures:
        print("\nConversion failures:", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)

    print("\nRunning cargo validation test...")
    val_errs = validate_all()
    if val_errs:
        print("Validation failures (first 40):", file=sys.stderr)
        for e in val_errs[:40]:
            print(f"  {e}", file=sys.stderr)
        return 1
    print("All shipped effects validate.")
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
