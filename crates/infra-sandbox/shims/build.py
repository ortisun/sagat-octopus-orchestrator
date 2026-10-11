# The builder of a Python script: checks that it compiles and defines run(input, octopus), and prints
# the result as a single line. It runs in a sandbox with no network.
import ast
import json
import sys

source = str(json.loads(sys.stdin.readline()).get("source", ""))


def say(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")


try:
    tree = ast.parse(source, filename="main.py")
    compile(source, "main.py", "exec", dont_inherit=True)
except SyntaxError as error:
    say({"op": "rejected", "message": f"{error.msg} (line {error.lineno})"})
    sys.exit(0)

defines_run = any(
    isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == "run"
    for node in tree.body
)
if not defines_run:
    say({"op": "rejected", "message": "the script must define a function run(input, octopus)"})
    sys.exit(0)
say({"op": "built", "entry": "main.py", "files": {"main.py": source}})
