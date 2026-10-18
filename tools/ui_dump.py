#!/usr/bin/env python3
"""Prints an action block of a prepared front-end screen as readable AVM1 assembly
(M9 analysis tool; the blocks use Flash's action opcode numbers).

    tools/ui_dump.py runtime/core_menu.linked.json OFFSET [--max N]
    tools/ui_dump.py runtime/core_menu.linked.json --list
"""
import argparse
import json

OPS = {0x04: "NextFrame", 0x05: "PrevFrame", 0x06: "Play", 0x07: "Stop", 0x0A: "Add", 0x0B: "Subtract", 0x0C: "Multiply",
       0x0D: "Divide", 0x0E: "Equals", 0x0F: "Less", 0x10: "And", 0x11: "Or", 0x12: "Not", 0x13: "StringEquals",
       0x14: "StringLength", 0x17: "Pop", 0x18: "ToInteger", 0x1C: "GetVariable", 0x1D: "SetVariable", 0x20: "SetTarget2",
       0x21: "StringAdd", 0x22: "GetProperty", 0x23: "SetProperty", 0x24: "CloneSprite", 0x25: "RemoveSprite", 0x26: "Trace",
       0x2B: "CastOp", 0x2C: "ImplementsOp", 0x30: "RandomNumber", 0x34: "GetTime", 0x3A: "Delete", 0x3B: "Delete2",
       0x3C: "DefineLocal", 0x3D: "CallFunction", 0x3E: "Return", 0x3F: "Modulo", 0x40: "NewObject", 0x41: "DefineLocal2",
       0x42: "InitArray", 0x43: "InitObject", 0x44: "TypeOf", 0x45: "TargetPath", 0x46: "Enumerate", 0x47: "Add2",
       0x48: "Less2", 0x49: "Equals2", 0x4A: "ToNumber", 0x4B: "ToString", 0x4C: "PushDuplicate", 0x4D: "StackSwap",
       0x4E: "GetMember", 0x4F: "SetMember", 0x50: "Increment", 0x51: "Decrement", 0x52: "CallMethod", 0x53: "NewMethod",
       0x54: "InstanceOf", 0x55: "Enumerate2", 0x60: "BitAnd", 0x61: "BitOr", 0x62: "BitXor", 0x63: "BitLShift",
       0x64: "BitRShift", 0x65: "BitURShift", 0x66: "StrictEquals", 0x67: "Greater", 0x68: "StringGreater", 0x69: "Extends",
       0x81: "GotoFrame", 0x83: "GetURL", 0x87: "StoreRegister", 0x88: "ConstantPool", 0x8B: "SetTarget", 0x8C: "GoToLabel",
       0x8E: "DefineFunction2", 0x94: "With", 0x96: "Push", 0x99: "Jump", 0x9A: "GetURL2", 0x9B: "DefineFunction",
       0x9D: "If", 0x9E: "Call", 0x9F: "GotoFrame2"}


def show(i, indent, out, limit):
    pad = "  " * indent
    name = OPS.get(i["opcode"], f"op{i['opcode']:#04x}")
    detail = ""
    if i.get("values"):
        detail = " " + ", ".join(repr(v["value"]) if v["kind"] in (1, 6, 7, 5) else f"<k{v['kind']}:{v['value']}>" for v in i["values"])
    if i.get("name") or i.get("parameters"):
        detail += f" {i.get('name') or '(anon)'}({', '.join(p['name'] or f'r{p['register']}' for p in i.get('parameters', []))})"
    if i.get("target") is not None:
        detail += f" -> {i['target']}"
    if i.get("operand") not in (None, 0, "", [], {}):
        detail += f" [{i['operand']}]"
    out.append(f"{pad}{i['offset']:>8} {name}{detail}")
    for b in i.get("body", []):
        show(b, indent + 1, out, limit)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("movie")
    ap.add_argument("offset", nargs="?")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--max", type=int, default=100000)
    a = ap.parse_args()
    d = json.load(open(a.movie))
    if a.list:
        for k, v in sorted(d["actions"].items(), key=lambda kv: int(kv[0])):
            print(k, len(v))
    else:
        out = []
        for i in d["actions"][a.offset]:
            show(i, 0, out, a.max)
        print("\n".join(out[: a.max]))
