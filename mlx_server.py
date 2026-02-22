#!/usr/bin/env python3
"""
MLX Model Server - Keeps model loaded in memory for fast inference
"""
import sys
import json
from mlx_lm import load, generate

class MLXServer:
    def __init__(self, model_id):
        print(f"Loading model: {model_id}...", file=sys.stderr, flush=True)
        self.model, self.tokenizer = load(model_id)
        print(f"Model loaded and ready!", file=sys.stderr, flush=True)

    def generate_response(self, messages, max_tokens=512, tools=None):
        """Generate a response given a messages list and optional tools.

        Uses the tokenizer's own chat template so the format is always
        correct regardless of which model is loaded (Qwen, Llama, Mistral…).
        """
        kwargs = dict(
            tokenize=False,
            add_generation_prompt=True,
        )
        if tools:
            kwargs["tools"] = tools

        try:
            prompt = self.tokenizer.apply_chat_template(messages, **kwargs)
        except Exception as e:
            # Some older tokenizers don't support the tools kwarg — retry without it
            if tools:
                kwargs.pop("tools", None)
                try:
                    prompt = self.tokenizer.apply_chat_template(messages, **kwargs)
                except Exception:
                    # Last-resort: build a plain text prompt
                    prompt = self._fallback_prompt(messages)
            else:
                prompt = self._fallback_prompt(messages)

        response = generate(
            self.model,
            self.tokenizer,
            prompt=prompt,
            max_tokens=max_tokens,
            verbose=False,
        )
        return response

    def _fallback_prompt(self, messages):
        """Plain-text fallback when apply_chat_template is unavailable."""
        parts = []
        for msg in messages:
            role = msg.get("role", "user")
            content = msg.get("content", "")
            parts.append(f"{role.capitalize()}: {content}")
        parts.append("Assistant:")
        return "\n\n".join(parts)


def main():
    if len(sys.argv) < 2:
        print("Usage: mlx_server.py <model_id>", file=sys.stderr)
        sys.exit(1)

    model_id = sys.argv[1]
    server = MLXServer(model_id)

    print("READY", flush=True)  # Signal that model is loaded

    # Read requests from stdin, one per line (each request is a JSON object)
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            max_tokens = request.get("max_tokens", 512)
            tools = request.get("tools")  # optional list of tool schemas

            if "messages" in request:
                response = server.generate_response(request["messages"], max_tokens, tools)
            else:
                # Legacy single-string prompt (no chat template applied)
                prompt = request.get("prompt", "")
                response = generate(
                    server.model,
                    server.tokenizer,
                    prompt=prompt,
                    max_tokens=max_tokens,
                    verbose=False,
                )

            print(json.dumps({"response": response, "error": None}), flush=True)

        except Exception as e:
            print(json.dumps({"response": None, "error": str(e)}), flush=True)


if __name__ == "__main__":
    main()
