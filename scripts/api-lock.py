#!/usr/bin/env python3
"""Lock the public API of every published Hopper crate, and name the version
a release needs.

cargo-semver-checks 0.50 passes a tree in which `AccountView::layout_id`
went from `Option<&[u8; 8]>` to `Option<[u8; 8]>` and
`DataFingerprint::capture` from `Self` to `Result<Self, ProgramError>`: it
has no lint for a changed return type. This script renders the signature of
every public item from the crate's rustdoc JSON into `audit/api/<package>.txt`.
A change to the public surface is then a diff in review, and `--verify`
fails CI when the lock no longer matches the code.

`--against-published` renders the version on crates.io the same way and
compares. A removed or changed item is a break; an added one is not. The
version a crate needs follows from its own changes and from its
dependencies: a crate whose public signatures mention a dependency that
needs a new minor version needs one too, because its users would otherwise
see two incompatible copies of the same type.

    python scripts/api-lock.py                          write every lock file
    python scripts/api-lock.py --verify                 fail if a lock file is stale
    python scripts/api-lock.py --against-published      the release plan
    python scripts/api-lock.py --against-published --strict
                                                        also fail when a manifest
                                                        version is lower than needed
    python scripts/api-lock.py --self-test              check the classifier
    -p/--package NAME                                   limit to these packages

Rustdoc JSON is unstable output. The script builds it with the toolchain
pinned in rust-toolchain.toml and `RUSTC_BOOTSTRAP=1`, as cargo-semver-checks
does, with each crate's default features.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCK_DIR = ROOT / "audit/api"
TARGET = ROOT / "target/api-lock"
HEADER = (
    "# Public API of {package}, rendered from rustdoc JSON by\n"
    "# scripts/api-lock.py with default features. Do not edit by hand: run the\n"
    "# script. A line that changes or disappears is a breaking change.\n"
)


# ---------------------------------------------------------------- packages


def cargo_metadata(cwd: Path) -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=cwd, capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)


def libraries() -> list[dict]:
    """Every publishable package with a library or proc-macro target."""
    found = []
    for package in cargo_metadata(ROOT)["packages"]:
        if package.get("publish") == []:
            continue
        for target in package["targets"]:
            if {"lib", "proc-macro", "rlib"} & set(target["kind"]):
                found.append({
                    "name": package["name"],
                    "version": package["version"],
                    "lib": target["name"].replace("-", "_"),
                    "dependencies": sorted({d["name"] for d in package["dependencies"] if d["kind"] is None}),
                })
                break
    return sorted(found, key=lambda p: p["name"])


def rustdoc_json(spec: str, lib: str, cwd: Path) -> dict:
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    args = [
        "cargo", "rustdoc", "-p", spec, "--lib", "--target-dir", str(TARGET),
        "--", "-Z", "unstable-options", "--output-format", "json",
    ]
    result = subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(result.stderr[-4000:])
        raise SystemExit(f"rustdoc failed for {spec}")
    return json.loads((TARGET / "doc" / f"{lib}.json").read_text(encoding="utf-8"))


# ---------------------------------------------------------------- rendering


def only(mapping: dict) -> tuple[str, object]:
    (key, value), = mapping.items()
    return key, value


class Api:
    """The public items of one crate, as `key -> line`.

    A key names the item by where it is defined, so moving a re-export does
    not look like removing an item. A line names it by its shortest public
    path, which is what a user writes.
    """

    def __init__(self, doc: dict, names: bool = True, canonical: bool = False):
        self.doc = doc
        self.index = doc["index"]
        self.paths = doc["paths"]
        self.names = names
        self.canonical = canonical
        root = self.item(doc["root"])
        self.crate = root["name"]
        self.public: dict[str, set[str]] = defaultdict(set)
        self.entries: dict[tuple, str] = {}
        # Types a new member breaks: enums a caller can match exhaustively,
        # structs a caller can build with a literal, and traits a caller
        # can implement. Keyed by where they are defined.
        self.exhaustive_enums: set[str] = set()
        self.literal_structs: set[str] = set()
        self.traits: set[str] = set()
        self.visited: set[str] = set()
        self.walk()
        self.render()

    def item(self, id_) -> dict | None:
        return self.index.get(str(id_))

    # -- walking the module tree

    def walk(self) -> None:
        queue = [(str(self.doc["root"]), self.crate)]
        self.visited.add(queue[0][0])
        while queue:
            module_id, module_path = queue.pop(0)
            module = self.item(module_id)
            self.visit_children(module["inner"]["module"]["items"], module_path, queue, 0)

    def visit_children(self, children: list, at: str, queue: list, depth: int) -> None:
        for child_id in children:
            child = self.item(child_id)
            if child is None or child["visibility"] != "public":
                continue
            kind, inner = only(child["inner"])
            if kind != "use":
                self.place(str(child_id), f"{at}::{child['name']}", queue)
                continue
            target_id = inner.get("id")
            target = self.item(target_id) if target_id is not None else None
            if inner["is_glob"]:
                if target is not None and "module" in target["inner"] and depth < 4:
                    self.visit_children(target["inner"]["module"]["items"], at, queue, depth + 1)
                elif target is not None and "enum" in target["inner"]:
                    for variant_id in target["inner"]["enum"]["variants"]:
                        variant = self.item(variant_id)
                        self.entries[("use", f"{at}::{variant['name']}")] = (
                            f"pub use {at}::{variant['name']} = {self.name_of(target_id)}::{variant['name']}"
                        )
                else:
                    self.entries[("use", f"{at}::*")] = f"pub use {at}::* = {self.external(inner)}::*"
                continue
            path = f"{at}::{inner['name']}"
            if target is None:
                self.entries[("use", path)] = f"pub use {path} = {self.external(inner)}"
            else:
                self.place(str(target_id), path, queue)

    def place(self, item_id: str, path: str, queue: list) -> None:
        self.public[item_id].add(path)
        item = self.item(item_id)
        if "module" in item["inner"] and item_id not in self.visited:
            self.visited.add(item_id)
            queue.append((item_id, path))

    def shortest(self, item_id: str) -> str:
        return min(self.public[item_id], key=lambda p: (p.count("::"), p))

    def defined_at(self, item_id: str) -> str:
        summary = self.paths.get(str(item_id))
        if summary:
            return "::".join(summary["path"])
        return self.shortest(item_id)

    def external(self, use: dict) -> str:
        summary = self.paths.get(str(use.get("id")))
        return "::".join(summary["path"]) if summary else use["source"]

    def name_of(self, item_id) -> str:
        """How a signature names an item: its public path in this crate, or
        its path in the crate that defines it."""
        key = str(item_id)
        summary = self.paths.get(key)
        if key in self.public and not (self.canonical and summary):
            return self.shortest(key)
        return "::".join(summary["path"]) if summary else "?"

    # -- types

    def ty(self, t) -> str:
        if t is None:
            return "_"
        if isinstance(t, str):
            return "_" if t == "infer" else t
        kind, v = only(t)
        if kind == "resolved_path":
            return self.path(v)
        if kind in ("generic", "primitive"):
            return v
        if kind == "borrowed_ref":
            lifetime = f"{v['lifetime']} " if v.get("lifetime") else ""
            return f"&{lifetime}{'mut ' if v['is_mutable'] else ''}{self.ty(v['type'])}"
        if kind == "raw_pointer":
            return f"*{'mut' if v['is_mutable'] else 'const'} {self.ty(v['type'])}"
        if kind == "tuple":
            inner = ", ".join(self.ty(x) for x in v)
            return f"({inner},)" if len(v) == 1 else f"({inner})"
        if kind == "slice":
            return f"[{self.ty(v)}]"
        if kind == "array":
            return f"[{self.ty(v['type'])}; {v['len']}]"
        if kind == "pat":
            return self.ty(v["type"])
        if kind == "impl_trait":
            return "impl " + " + ".join(self.bound(b) for b in v)
        if kind == "dyn_trait":
            parts = [self.hrtb(p.get("generic_params") or []) + self.path(p["trait"]) for p in v["traits"]]
            if v.get("lifetime"):
                parts.append(v["lifetime"])
            return "dyn " + " + ".join(parts)
        if kind == "qualified_path":
            self_type = self.ty(v["self_type"])
            args = self.args(v.get("args"))
            if v.get("trait"):
                return f"<{self_type} as {self.path(v['trait'])}>::{v['name']}{args}"
            return f"{self_type}::{v['name']}{args}"
        if kind == "function_pointer":
            header = v["header"]
            quals = ("unsafe " if header.get("is_unsafe") else "") + self.abi(header.get("abi"))
            inputs = ", ".join(self.ty(t) for _, t in v["sig"]["inputs"])
            output = v["sig"].get("output")
            ret = f" -> {self.ty(output)}" if output is not None else ""
            return f"{self.hrtb(v.get('generic_params') or [])}{quals}fn({inputs}){ret}"
        return f"?{kind}"

    def path(self, p: dict) -> str:
        name = self.name_of(p["id"]) if p.get("id") is not None else p["path"]
        if name == "?":
            name = p["path"]
        return name + self.args(p.get("args"))

    def args(self, a) -> str:
        if not a:
            return ""
        kind, v = only(a)
        if kind == "parenthesized":
            out = "(" + ", ".join(self.ty(x) for x in v["inputs"]) + ")"
            if v.get("output") is not None:
                out += " -> " + self.ty(v["output"])
            return out
        if kind != "angle_bracketed":
            return "(..)"
        parts = []
        for arg in v["args"]:
            if isinstance(arg, str):
                parts.append("_")
                continue
            arg_kind, value = only(arg)
            if arg_kind == "lifetime":
                parts.append(value)
            elif arg_kind == "type":
                parts.append(self.ty(value))
            elif arg_kind == "const":
                parts.append(value.get("expr") or value.get("value") or "_")
            else:
                parts.append("_")
        for constraint in v.get("constraints", []):
            name = constraint["name"] + self.args(constraint.get("args"))
            binding_kind, binding = only(constraint["binding"])
            if binding_kind == "equality":
                term_kind, term = only(binding)
                rhs = self.ty(term) if term_kind == "type" else (term.get("expr") or "_")
                parts.append(f"{name} = {rhs}")
            else:
                parts.append(f"{name}: " + " + ".join(self.bound(b) for b in binding))
        return "<" + ", ".join(parts) + ">" if parts else ""

    def bound(self, b) -> str:
        if isinstance(b, str):
            return b
        kind, v = only(b)
        if kind == "trait_bound":
            modifier = {"maybe": "?", "maybe_const": "~const "}.get(v.get("modifier"), "")
            return self.hrtb(v.get("generic_params") or []) + modifier + self.path(v["trait"])
        if kind == "outlives":
            return v
        if kind == "use":
            names = [only(x)[1] if isinstance(x, dict) else str(x) for x in v]
            return "use<" + ", ".join(names) + ">"
        return f"?{kind}"

    def hrtb(self, params: list) -> str:
        names = [p["name"] for p in params]
        return f"for<{', '.join(names)}> " if names else ""

    def generics(self, g: dict) -> tuple[str, str]:
        params = []
        for param in g.get("params", []):
            kind, v = only(param["kind"])
            if kind == "lifetime":
                text = param["name"]
                if v.get("outlives"):
                    text += ": " + " + ".join(v["outlives"])
            elif kind == "type":
                if v.get("is_synthetic"):
                    continue
                text = param["name"]
                if v.get("bounds"):
                    text += ": " + " + ".join(self.bound(b) for b in v["bounds"])
                if v.get("default") is not None:
                    text += " = " + self.ty(v["default"])
            else:
                text = f"const {param['name']}: {self.ty(v['type'])}"
                if v.get("default"):
                    text += f" = {v['default']}"
            params.append(text)
        predicates = []
        for predicate in g.get("where_predicates", []):
            kind, v = only(predicate)
            if kind == "bound_predicate":
                bounds = " + ".join(self.bound(b) for b in v["bounds"])
                predicates.append(f"{self.hrtb(v.get('generic_params') or [])}{self.ty(v['type'])}: {bounds}")
            elif kind == "lifetime_predicate":
                predicates.append(f"{v['lifetime']}: " + " + ".join(v["outlives"]))
            elif kind == "eq_predicate":
                term_kind, term = only(v["rhs"])
                rhs = self.ty(term) if term_kind == "type" else "_"
                predicates.append(f"{self.ty(v['lhs'])} = {rhs}")
        return (
            "<" + ", ".join(params) + ">" if params else "",
            " where " + ", ".join(predicates) if predicates else "",
        )

    def abi(self, abi) -> str:
        if abi in (None, "Rust"):
            return ""
        if isinstance(abi, str):
            return f'extern "{abi}" '
        name, _ = only(abi)
        return f'extern "{name}" '

    def function(self, name: str, f: dict) -> str:
        header = f["header"]
        quals = (
            ("const " if header.get("is_const") else "")
            + ("async " if header.get("is_async") else "")
            + ("unsafe " if header.get("is_unsafe") else "")
            + self.abi(header.get("abi"))
        )
        params, where = self.generics(f["generics"])
        inputs = []
        for arg_name, arg_type in f["sig"]["inputs"]:
            if arg_name == "self":
                inputs.append(self.self_param(arg_type))
            elif self.names:
                inputs.append(f"{arg_name}: {self.ty(arg_type)}")
            else:
                inputs.append(self.ty(arg_type))
        if f["sig"].get("is_c_variadic"):
            inputs.append("...")
        output = f["sig"].get("output")
        ret = f" -> {self.ty(output)}" if output is not None else ""
        return f"{quals}fn {name}{params}({', '.join(inputs)}){ret}{where}"

    def self_param(self, t: dict) -> str:
        if t == {"generic": "Self"}:
            return "self"
        if "borrowed_ref" in t and t["borrowed_ref"]["type"] == {"generic": "Self"}:
            ref = t["borrowed_ref"]
            lifetime = f"{ref['lifetime']} " if ref.get("lifetime") else ""
            return f"&{lifetime}{'mut ' if ref['is_mutable'] else ''}self"
        return f"self: {self.ty(t)}"

    # -- items

    def render(self) -> None:
        for item_id in sorted(self.public, key=lambda i: self.shortest(i)):
            item = self.item(item_id)
            kind, inner = only(item["inner"])
            for path in sorted(self.public[item_id]):
                self.entries[("decl", path)] = self.declaration(item, kind, inner, path)
            self.members(item_id, item, kind, inner)

    def attrs(self, item: dict) -> str:
        text = json.dumps(item.get("attrs", []))
        return "#[non_exhaustive] " if "non_exhaustive" in text else ""

    def declaration(self, item: dict, kind: str, inner, path: str) -> str:
        if kind == "module":
            return f"pub mod {path}"
        if kind == "function":
            return "pub " + self.function(path, inner)
        if kind in ("struct", "union"):
            params, where = self.generics(inner["generics"])
            shape = ""
            if kind == "struct":
                shape_kind = inner["kind"]
                if shape_kind == "unit":
                    shape = ";"
                elif "tuple" in shape_kind:
                    fields = [
                        ("pub " + self.ty(self.item(f)["inner"]["struct_field"])) if f is not None else "_"
                        for f in shape_kind["tuple"]
                    ]
                    shape = "(" + ", ".join(fields) + ")"
                else:
                    shape = " { .. }" if shape_kind["plain"].get("has_stripped_fields") else " {}"
            return f"{self.attrs(item)}pub {kind} {path}{params}{where}{shape}"
        if kind == "enum":
            params, where = self.generics(inner["generics"])
            rest = " { .. }" if inner.get("has_stripped_variants") else ""
            return f"{self.attrs(item)}pub enum {path}{params}{where}{rest}"
        if kind == "trait":
            params, where = self.generics(inner["generics"])
            bounds = inner.get("bounds") or []
            supers = ": " + " + ".join(self.bound(b) for b in bounds) if bounds else ""
            unsafe = "unsafe " if inner.get("is_unsafe") else ""
            dyn = " [dyn compatible]" if inner.get("is_dyn_compatible") else ""
            return f"pub {unsafe}trait {path}{params}{supers}{where}{dyn}"
        if kind == "constant":
            const = inner.get("const") or {}
            value = const.get("value") or const.get("expr") or "_"
            return f"pub const {path}: {self.ty(inner['type'])} = {value}"
        if kind == "static":
            mutable = "mut " if inner.get("is_mutable") else ""
            return f"pub static {mutable}{path}: {self.ty(inner['type'])}"
        if kind == "type_alias":
            params, where = self.generics(inner["generics"])
            return f"pub type {path}{params} = {self.ty(inner['type'])}{where}"
        if kind == "macro":
            arms = " ".join(inner.split())
            return f"macro {path}! {arms}"
        if kind == "proc_macro":
            helpers = inner.get("helpers") or []
            shape = {"bang": f"{path}!", "attr": f"#[{path}]", "derive": f"#[derive({path})]"}.get(inner["kind"], path)
            extra = f" helpers({', '.join(helpers)})" if helpers else ""
            return f"proc_macro {shape}{extra}"
        if kind == "trait_alias":
            return f"pub trait {path} = .."
        return f"pub {kind} {path}"

    def members(self, item_id: str, item: dict, kind: str, inner) -> None:
        defined = self.defined_at(item_id)
        path = defined if self.canonical else self.shortest(item_id)
        if kind == "struct":
            shape = inner["kind"]
            if (
                isinstance(shape, dict)
                and "plain" in shape
                and not shape["plain"].get("has_stripped_fields")
                and not self.attrs(item)
            ):
                self.literal_structs.add(defined)
            if isinstance(shape, dict) and "plain" in shape:
                for field_id in shape["plain"]["fields"]:
                    field = self.item(field_id)
                    if field and field["visibility"] == "public":
                        self.entries[("field", f"{defined}::{field['name']}")] = (
                            f"pub {path}::{field['name']}: {self.ty(field['inner']['struct_field'])}"
                        )
            self.impls(inner.get("impls", []), path, defined)
        elif kind == "union":
            for field_id in inner.get("fields", []):
                field = self.item(field_id)
                if field and field["visibility"] == "public":
                    self.entries[("field", f"{defined}::{field['name']}")] = (
                        f"pub {path}::{field['name']}: {self.ty(field['inner']['struct_field'])}"
                    )
            self.impls(inner.get("impls", []), path, defined)
        elif kind == "enum":
            if not self.attrs(item):
                self.exhaustive_enums.add(defined)
            for variant_id in inner["variants"]:
                variant = self.item(variant_id)
                self.entries[("variant", f"{defined}::{variant['name']}")] = self.variant(path, variant)
            self.impls(inner.get("impls", []), path, defined)
        elif kind == "trait":
            self.traits.add(defined)
            for member_id in inner["items"]:
                member = self.item(member_id)
                if member is None:
                    continue
                self.entries[("trait item", f"{defined}::{member['name']}")] = self.trait_item(path, member)
            for impl_id in inner.get("implementations", []):
                self.impl_line(impl_id)

    def variant(self, path: str, variant: dict) -> str:
        v = variant["inner"]["variant"]
        name = f"{path}::{variant['name']}"
        shape = v["kind"]
        if isinstance(shape, dict) and "tuple" in shape:
            fields = [self.ty(self.item(f)["inner"]["struct_field"]) if f is not None else "_" for f in shape["tuple"]]
            name += "(" + ", ".join(fields) + ")"
        elif isinstance(shape, dict) and "struct" in shape:
            fields = []
            for f in shape["struct"]["fields"]:
                field = self.item(f)
                fields.append(f"{field['name']}: {self.ty(field['inner']['struct_field'])}")
            if shape["struct"].get("has_stripped_fields"):
                fields.append("..")
            name += " { " + ", ".join(fields) + " }"
        discriminant = v.get("discriminant")
        if discriminant:
            name += f" = {discriminant.get('value') or discriminant.get('expr')}"
        return name

    def trait_item(self, path: str, member: dict) -> str:
        kind, inner = only(member["inner"])
        name = f"{path}::{member['name']}"
        if kind == "function":
            body = " { .. }" if inner.get("has_body") else ";"
            return self.function(name, inner) + body
        if kind == "assoc_const":
            default = f" = {inner['value']}" if inner.get("value") not in (None, "_") else ""
            return f"const {name}: {self.ty(inner['type'])}{default}"
        if kind == "assoc_type":
            bounds = inner.get("bounds") or []
            text = f"type {name}"
            if bounds:
                text += ": " + " + ".join(self.bound(b) for b in bounds)
            if inner.get("type") is not None:
                text += " = " + self.ty(inner["type"])
            return text
        return f"{kind} {name}"

    def impls(self, impl_ids: list, path: str, defined: str) -> None:
        for impl_id in impl_ids:
            impl = self.item(impl_id)
            if impl is None:
                continue
            inner = impl["inner"]["impl"]
            if inner.get("blanket_impl") is not None:
                continue
            if inner.get("trait") is not None:
                self.impl_line(impl_id)
                continue
            for member_id in inner["items"]:
                member = self.item(member_id)
                if member is None or member["visibility"] != "public":
                    continue
                kind, value = only(member["inner"])
                name = f"{path}::{member['name']}"
                key = f"{defined}::{member['name']}"
                if kind == "function":
                    self.entries[("fn", key)] = "pub " + self.function(name, value)
                elif kind == "assoc_const":
                    default = f" = {value['value']}" if value.get("value") not in (None, "_") else ""
                    self.entries[("const", key)] = f"pub const {name}: {self.ty(value['type'])}{default}"
                elif kind == "assoc_type":
                    self.entries[("type", key)] = f"pub type {name} = {self.ty(value.get('type'))}"

    def impl_line(self, impl_id) -> None:
        impl = self.item(impl_id)
        if impl is None:
            return
        inner = impl["inner"]["impl"]
        if inner.get("blanket_impl") is not None or inner.get("trait") is None:
            return
        params, where = self.generics(inner["generics"])
        negative = "!" if inner.get("is_negative") else ""
        unsafe = "unsafe " if inner.get("is_unsafe") else ""
        line = (
            f"{unsafe}impl{params} {negative}{self.path(inner['trait'])} "
            f"for {self.ty(inner['for'])}{where}"
        )
        if inner.get("is_synthetic"):
            line += " [auto]"
        self.entries[("impl", line)] = line

    def lines(self) -> list[str]:
        return sorted(set(self.entries.values()))


# ---------------------------------------------------------------- commands


def lock_text(package: str, api: Api) -> str:
    return HEADER.format(package=package) + "\n".join(api.lines()) + "\n"


def current_api(package: dict, names: bool = True, canonical: bool = False) -> Api:
    doc = rustdoc_json(f"{package['name']}@{package['version']}", package["lib"], ROOT)
    return Api(doc, names, canonical)


def write_or_verify(packages: list[dict], verify: bool) -> int:
    LOCK_DIR.mkdir(parents=True, exist_ok=True)
    stale = []
    for package in packages:
        text = lock_text(package["name"], current_api(package))
        path = LOCK_DIR / f"{package['name']}.txt"
        have = path.read_text(encoding="utf-8").replace("\r\n", "\n") if path.exists() else None
        if verify:
            if have != text:
                stale.append(package["name"])
                old = set((have or "").splitlines())
                new = set(text.splitlines())
                for line in sorted(old - new)[:20]:
                    print(f"  - {line}")
                for line in sorted(new - old)[:20]:
                    print(f"  + {line}")
        else:
            path.write_text(text, encoding="utf-8", newline="\n")
            print(f"{package['name']}: {text.count(chr(10)) - 3} items")
    if verify:
        known = {p["name"] for p in packages}
        if not any(p for p in known if p not in {q["name"] for q in packages}):
            orphans = [
                p.stem for p in LOCK_DIR.glob("*.txt")
                if p.stem not in {q["name"] for q in libraries()}
            ]
            for orphan in orphans:
                print(f"  lock file for a package that is not published: {orphan}")
                stale.append(orphan)
        if stale:
            print("public API changed in: " + ", ".join(stale) + " (review the diff, then run scripts/api-lock.py)")
            return 1
        print(f"public API lock is current: {len(packages)} packages")
    return 0


def published_version(name: str) -> str | None:
    request = urllib.request.Request(
        f"https://crates.io/api/v1/crates/{name}",
        headers={"User-Agent": "hopper-api-lock (https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework)"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)["crate"]["max_version"]
    except Exception:
        return None


def version_tuple(version: str) -> tuple[int, int, int]:
    core = version.split("-")[0].split("+")[0]
    major, minor, patch = (int(x) for x in core.split("."))
    return major, minor, patch


def next_version(published: str, level: str) -> str:
    major, minor, patch = version_tuple(published)
    if level in ("breaking", "review"):
        return f"0.{minor + 1}.0" if major == 0 else f"{major + 1}.0.0"
    if level == "additive":
        return f"0.{minor}.{patch + 1}" if major == 0 else f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def baseline_apis(packages: list[dict]) -> dict[str, Api]:
    """Render the crates.io version of each package, from one scratch
    project that depends on all of them."""
    wanted = {p["name"]: published_version(p["name"]) for p in packages}
    wanted = {name: version for name, version in wanted.items() if version}
    scratch = Path(tempfile.mkdtemp(prefix="hopper-api-baseline-"))
    try:
        dependencies = "\n".join(f'{name} = "={version}"' for name, version in sorted(wanted.items()))
        (scratch / "Cargo.toml").write_text(
            '[package]\nname = "hopper-api-baseline"\nversion = "0.0.0"\nedition = "2021"\n'
            f"publish = false\n\n[dependencies]\n{dependencies}\n\n[workspace]\n",
            encoding="utf-8",
        )
        (scratch / "src").mkdir()
        (scratch / "src/lib.rs").write_text("", encoding="utf-8")
        apis = {}
        for package in packages:
            version = wanted.get(package["name"])
            if version is None:
                continue
            doc = rustdoc_json(f"{package['name']}@{version}", package["lib"], scratch)
            apis[package["name"]] = Api(doc, names=False, canonical=True)
        return apis
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


LIFETIME = re.compile(r"'[A-Za-z_][A-Za-z0-9_]*\b\s*|<'_>|'_\s*")


def macro_arms(text: str) -> list[str]:
    body = text.split(" { ", 1)[1] if " { " in text else text
    return [arm.strip() for arm in body.split(" => { ... };")[:-1]]


def classify(old: str, new: str) -> tuple[str, str]:
    """How a change to one item's line affects its users: `breaking`,
    `review` (compatible for the usual uses, with a documented edge), or
    `additive`, and why."""
    if old.startswith("macro ") and new.startswith("macro "):
        if set(macro_arms(old)) <= set(macro_arms(new)):
            return "additive", "macro gained a rule; every earlier rule is unchanged"
        return "breaking", "macro rules changed"
    if new.replace("const fn ", "fn ", 1) == old:
        return "additive", "now const"
    if LIFETIME.sub("", old) == LIFETIME.sub("", new):
        return "review", "only lifetimes changed"
    if re.match(r"(#\[non_exhaustive\] )?pub (enum|struct) ", old) and new.startswith("pub type "):
        return "review", "now a type alias; a glob import of its variants or a trait impl on both names breaks"
    return "breaking", "signature or value changed"


def required_trait_item(text: str) -> bool:
    """A trait item an implementor has to write: a method without a body,
    or an associated const or type without a default."""
    if " fn " in f" {text}":
        return text.endswith(";")
    return " = " not in text


def breaking_addition(key: tuple, text: str, before: "Api") -> str | None:
    """Why adding this member breaks callers of the published version, or
    `None` when it does not."""
    kind, path = key
    parent = path.rsplit("::", 1)[0]
    if kind == "variant" and parent in before.exhaustive_enums:
        return "variant added to an enum callers match exhaustively"
    if kind == "field" and parent in before.literal_structs:
        return "field added to a struct callers build with a literal"
    if kind == "trait item" and parent in before.traits and required_trait_item(text):
        return "required item added to a trait callers implement"
    return None


def against_published(packages: list[dict], strict: bool) -> int:
    baselines = baseline_apis(packages)
    plan = {}
    for package in packages:
        name = package["name"]
        current = current_api(package, names=False, canonical=True)
        published = published_version(name)
        if name not in baselines or published is None:
            plan[name] = {"published": None, "level": "new", "why": ["not on crates.io"], "api": current}
            continue
        old, new = baselines[name].entries, current.entries
        changes = []
        aliased = []
        for key in sorted(old.keys() & new.keys()):
            if old[key] != new[key]:
                level, reason = classify(old[key], new[key])
                changes.append((level, reason, old[key], new[key]))
                if reason.startswith("now a type alias"):
                    aliased.append(key[1])

        # Members and impls of a type that became an alias of another are
        # reached through the alias now; the alias entry covers them.
        def through_alias(text: str) -> bool:
            return any(f"{path}::" in text or f"for {path}" in text for path in aliased)

        removed = sorted(old[k] for k in old.keys() - new.keys() if not through_alias(old[k]))
        added = []
        for key in sorted(new.keys() - old.keys()):
            reason = breaking_addition(key, new[key], baselines[name])
            if reason:
                changes.append(("breaking", reason, "(absent)", new[key]))
            else:
                added.append(new[key])
        levels = {c[0] for c in changes}
        if removed or "breaking" in levels:
            level = "breaking"
        elif "review" in levels:
            level = "review"
        elif added or levels:
            level = "additive"
        else:
            level = "none"
        why = []
        for tag in ("breaking", "review", "additive"):
            count = sum(1 for c in changes if c[0] == tag)
            if count:
                why.append(f"{count} {tag} change{'s' if count > 1 else ''}")
        if removed:
            why.append(f"{len(removed)} removed")
        if added:
            why.append(f"{len(added)} added")
        plan[name] = {
            "published": published, "level": level, "why": why, "api": current,
            "removed": removed, "changes": changes, "added": added,
        }

    # A crate that names a dependency's types in its own signatures breaks
    # when that dependency does.
    libs = {p["name"]: p["lib"] for p in packages}
    deps = {p["name"]: [d for d in p["dependencies"] if d in libs] for p in packages}
    changed_level = True
    while changed_level:
        changed_level = False
        for name, entry in plan.items():
            if entry["level"] in ("breaking", "review", "new"):
                continue
            text = "\n".join(entry["api"].entries.values())
            for dep in deps[name]:
                if plan.get(dep, {}).get("level") in ("breaking", "review") and f"{libs[dep]}::" in text:
                    entry["level"] = "breaking"
                    entry["why"].append(f"exposes {dep} types")
                    changed_level = True
                    break

    failures = 0
    manifest = {p["name"]: p["version"] for p in packages}
    print(f"{'package':<26} {'crates.io':<10} {'manifest':<10} {'needs':<10} why")
    for name in sorted(plan):
        entry = plan[name]
        published = entry["published"]
        if published is None:
            print(f"{name:<26} {'-':<10} {manifest[name]:<10} {'-':<10} not on crates.io")
            continue
        if entry["level"] == "none":
            needs = "-"
            status = ""
        else:
            needs = next_version(published, entry["level"])
            status = "" if version_tuple(manifest[name]) >= version_tuple(needs) else "  (manifest too low)"
            if status:
                failures += 1
        print(f"{name:<26} {published:<10} {manifest[name]:<10} {needs:<10} {', '.join(entry['why']) or 'no public API change'}{status}")
    for name in sorted(plan):
        entry = plan[name]
        if entry.get("removed") or entry.get("changes"):
            print(f"\n{name} {entry['published']} -> tree")
            for level, reason, old, new in entry["changes"]:
                print(f"  {level}: {reason}\n    was {old}\n    now {new}")
            for line in entry["removed"]:
                print(f"  breaking: removed\n    was {line}")
    return 1 if strict and failures else 0


def self_test() -> int:
    """The classifier against the changes it exists to catch."""
    import unittest

    class Classify(unittest.TestCase):
        def test_a_changed_return_type_is_a_break(self):
            old = "pub fn a::AccountView::layout_id(&self) -> core::option::Option<&[u8; 8]>"
            new = "pub fn a::AccountView::layout_id(&self) -> core::option::Option<[u8; 8]>"
            self.assertEqual(classify(old, new)[0], "breaking")

        def test_a_changed_constant_is_a_break(self):
            old = "pub const a::hash::MAX_HASH_SEGMENTS: usize = 16usize"
            new = "pub const a::hash::MAX_HASH_SEGMENTS: usize = 20_000usize"
            self.assertEqual(classify(old, new)[0], "breaking")

        def test_becoming_const_is_additive(self):
            old = "pub fn a::hint::likely(bool) -> bool"
            self.assertEqual(classify(old, old.replace("fn ", "const fn ", 1))[0], "additive")

        def test_a_macro_that_only_gained_rules_is_additive(self):
            old = "macro a::m! macro_rules! m { () => { ... }; }"
            new = "macro a::m! macro_rules! m { () => { ... }; (heap = $len:expr) => { ... }; }"
            self.assertEqual(classify(old, new)[0], "additive")
            self.assertEqual(classify(new, old)[0], "breaking")

        def test_a_longer_output_lifetime_needs_review(self):
            old = "pub fn a::FieldRef::as_address(&self) -> core::result::Result<&[u8; 32], E>"
            new = "pub fn a::FieldRef::as_address(&self) -> core::result::Result<&'a [u8; 32], E>"
            self.assertEqual(classify(old, new)[0], "review")

        def test_an_enum_that_became_an_alias_needs_review(self):
            old = "pub enum a::token::MintProgram"
            new = "pub type a::token::MintProgram = a::token::TokenProgram"
            self.assertEqual(classify(old, new)[0], "review")

        def test_required_trait_items(self):
            self.assertTrue(required_trait_item("fn a::T::f(&self) -> u8;"))
            self.assertFalse(required_trait_item("fn a::T::f(&self) -> u8 { .. }"))
            self.assertTrue(required_trait_item("const a::T::SIZE: usize"))
            self.assertFalse(required_trait_item("const a::T::SIZE: usize = 8"))
            self.assertTrue(required_trait_item("type a::T::Out"))

        def test_versions(self):
            self.assertEqual(next_version("0.4.4", "breaking"), "0.5.0")
            self.assertEqual(next_version("0.4.4", "review"), "0.5.0")
            self.assertEqual(next_version("0.4.0", "additive"), "0.4.1")
            self.assertEqual(next_version("1.2.3", "breaking"), "2.0.0")
            self.assertEqual(next_version("1.2.3", "additive"), "1.3.0")

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Classify)
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    return 0 if result.wasSuccessful() else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--against-published", action="store_true")
    parser.add_argument("--strict", action="store_true")
    parser.add_argument("-p", "--package", action="append", default=[])
    args = parser.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    if args.self_test:
        return self_test()

    packages = libraries()
    if args.package:
        unknown = set(args.package) - {p["name"] for p in packages}
        if unknown:
            print("not a published library: " + ", ".join(sorted(unknown)))
            return 1
        packages = [p for p in packages if p["name"] in args.package]
    if args.against_published:
        return against_published(packages, args.strict)
    return write_or_verify(packages, args.verify)


if __name__ == "__main__":
    sys.exit(main())
