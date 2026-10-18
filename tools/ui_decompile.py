#!/usr/bin/env python3
"""Pseudo-code for a prepared front-end action block (M9 analysis tool): a symbolic
stack over the compact AVM1 opcodes the VM implements (crates/skate-ui/src/apt_vm.rs).
Readable enough to follow a screen class; jumps are shown as gotos.

    tools/ui_decompile.py runtime/core_menu.linked.json OFFSET [--find NAME]
"""
import argparse
import json


def render(block, pool, indent, out):
    stack = []
    pad = "  " * indent
    regs = {}

    def pop():
        return stack.pop() if stack else "<empty>"

    def emit(text):
        out.append(f"{pad}{text}")

    for i in block:
        op, operand = i["opcode"], i.get("operand")
        label = f"{i['offset']:>8}"
        if op == 0x88:
            pool[:] = [str(v["value"]) if v["kind"] != 1 else v["value"] for v in i["values"]]
        elif op == 0x96:
            for v in i["values"]:
                stack.append(repr(v["value"]))
        elif op == 0xa1:
            stack.append(repr(operand))
        elif op in (0xa2, 0xa3):
            stack.append(repr(pool[operand]) if operand < len(pool) else f"const{operand}")
        elif op == 0xa4:
            stack.append(operand)
        elif op == 0xae:
            stack.append(pool[operand])
        elif op == 0xa5:
            stack.append(f"{pop()}.{operand}")
        elif op == 0xaf:
            stack.append(f"{pop()}.{pool[operand]}")
        elif op == 0x4e:
            name = pop(); o = pop(); stack.append(f"{o}[{name}]")
        elif op == 0xa6:
            emit(f"{operand} = {pop()}")
        elif op == 0x1d:
            v = pop(); k = pop(); emit(f"{k.strip(chr(39))} = {v}")
        elif op == 0x3c:
            v = pop(); k = pop(); emit(f"var {k.strip(chr(39))} = {v}")
        elif op == 0x4f:
            v = pop(); k = pop(); o = pop(); emit(f"{o}.{k.strip(chr(39))} = {v}")
        elif op == 0x70: stack.append("this")
        elif op == 0x71: stack.append("_global")
        elif op == 0x59: stack.append("0")
        elif op == 0x5a: stack.append("1")
        elif op == 0x73: stack.append("true")
        elif op == 0x74: stack.append("false")
        elif op in (0x75, 0x76): stack.append("undefined")
        elif op in (0xb5, 0xb6, 0xb7):
            bits = {0xb5: 8, 0xb6: 16, 0xb7: 32}[op]
            v = operand & ((1 << bits) - 1)
            stack.append(str(v - (1 << bits) if v >= 1 << (bits - 1) else v))
        elif op == 0xb4: stack.append(f"float({operand})")
        elif op == 0xb9: stack.append(f"r{operand}")
        elif op == 0x87:
            emit(f"r{operand} = {stack[-1] if stack else '?'}")
        elif op in (0x47, 0x0b, 0x0c, 0x0d, 0x48, 0x67, 0x66, 0x49, 0x3f, 0x10, 0x11):
            sym = {0x47: "+", 0x0b: "-", 0x0c: "*", 0x0d: "/", 0x48: "<", 0x67: ">", 0x66: "===", 0x49: "==", 0x3f: "%", 0x10: "&&", 0x11: "||"}[op]
            b = pop(); a = pop(); stack.append(f"({a} {sym} {b})")
        elif op == 0x12: stack.append(f"!{pop()}")
        elif op in (0x50, 0x51): stack.append(f"({pop()}{'+' if op == 0x50 else '-'}1)")
        elif op == 0x4c: stack.append(stack[-1] if stack else "?")
        elif op == 0x17:
            e = pop()
            if "(" in e: emit(e)
        elif op in (0x52, 0x5d, 0x3d, 0xb0, 0xb1, 0xb2, 0xb3):
            method = pool[operand] if op in (0xb0, 0xb1, 0xb2, 0xb3) else pop().strip("'")
            obj = "_global" if op in (0x3d, 0xb0, 0xb1) else pop()
            n = int(float(pop()))
            args = [pop() for _ in range(n)]
            call = f"{obj}.{method}({', '.join(args)})" if obj != "_global" else f"{method}({', '.join(args)})"
            if op in (0xb0, 0xb2, 0x5d): emit(call)
            elif op in (0xb1, 0xb3): emit(f"return {call}")
            else: stack.append(call)
        elif op == 0x40:
            name = pop(); n = int(float(pop())); args = [pop() for _ in range(n)]
            stack.append(f"new {name.strip(chr(39))}({', '.join(args)})")
        elif op in (0x8e, 0x9b):
            params = ", ".join(p["name"] or f"r{p['register']}" for p in i.get("parameters", []))
            sub = []
            render(i.get("body", []), list(pool), indent + 1, sub)
            header = f"function {i.get('name') or ''}({params}) {{"
            text = "\n".join([f"{pad}{header}"] + sub + [f"{pad}}}"])
            if i.get("name"): out.append(text)
            else: stack.append("\n" + text.replace("\n", "\n"))
        elif op == 0x69:
            sup = pop(); sub = pop(); emit(f"{sub}.extends({sup})")
        elif op in (0x99, 0x9d, 0xb8):
            cond = "" if op == 0x99 else f"if ({'!' if op == 0xb8 else ''}{pop()}) "
            emit(f"{cond}goto {i.get('target')}")
        elif op == 0x3e: emit(f"return {pop() if stack else ''}")
        elif op in (0x06, 0x07): emit("play()" if op == 6 else "stop()")
        elif op == 0x26: emit(f"trace({pop()})")
        elif op == 0x3a:
            k = pop(); o = pop(); stack.append(f"delete {o}.{k}")
        elif op == 0:
            pass
        else:
            emit(f"/* op{op:#04x} {operand} */")
    return out


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("movie"); ap.add_argument("offset")
    ap.add_argument("--find")
    a = ap.parse_args()
    d = json.load(open(a.movie))
    out = render(d["actions"][a.offset], [], 0, [])
    text = "\n".join(out)
    if a.find:
        lines = text.split("\n")
        hits = [n for n, l in enumerate(lines) if a.find in l]
        for n in hits[:3]:
            print("\n".join(lines[max(0, n - 2): n + 40])); print("....")
    else:
        print(text)
