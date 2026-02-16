#!/usr/bin/env python3
"""
MLX-LM HTTP server for forge-rs
Provides OpenAI-compatible API for local MLX language models
"""

import sys
import json
import argparse
import traceback
from typing import List, Dict, Any, Optional
from pathlib import Path
import asyncio
from datetime import datetime

try:
    from fastapi import FastAPI, HTTPException
    from fastapi.responses import StreamingResponse
    import uvicorn
    from pydantic import BaseModel
    FASTAPI_AVAILABLE = True
except ImportError:
    FASTAPI_AVAILABLE = False

try:
    import mlx.core as mx
    from mlx_lm import load, generate, stream_generate
    from mlx_lm.utils import load_config
    MLX_LM_AVAILABLE = True
except ImportError as e:
    MLX_LM_AVAILABLE = False
    MLX_LM_IMPORT_ERROR = str(e)


# OpenAI-compatible request/response models
class Message(BaseModel):
    role: str
    content: str

class ChatCompletionRequest(BaseModel):
    model: str
    messages: List[Message]
    max_tokens: Optional[int] = 512
    temperature: Optional[float] = 0.7
    top_p: Optional[float] = 0.9
    stream: Optional[bool] = False

class Choice(BaseModel):
    index: int
    message: Message
    finish_reason: str

class Usage(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int

class ChatCompletionResponse(BaseModel):
    id: str
    object: str = "chat.completion"
    created: int
    model: str
    choices: List[Choice]
    usage: Usage

class StreamChoice(BaseModel):
    index: int
    delta: Dict[str, Any]
    finish_reason: Optional[str] = None

class StreamResponse(BaseModel):
    id: str
    object: str = "chat.completion.chunk"
    created: int
    model: str
    choices: List[StreamChoice]


class MLXServer:
    def __init__(self, model_name: str, host: str = "127.0.0.1", port: int = 8000):
        self.model_name = model_name
        self.host = host
        self.port = port
        self.model = None
        self.tokenizer = None
        self.app = FastAPI(title="MLX-LM Server", version="1.0.0")
        self.setup_routes()
    
    def load_model(self):
        """Load the MLX language model"""
        if not MLX_LM_AVAILABLE:
            print(f"Warning: MLX-LM not available ({MLX_LM_IMPORT_ERROR}), using mock responses", file=sys.stderr)
            return
            
        try:
            print(f"Loading MLX model: {self.model_name}")
            self.model, self.tokenizer = load(self.model_name)
            print("Model loaded successfully!")
        except Exception as e:
            print(f"Failed to load MLX model '{self.model_name}': {e}", file=sys.stderr)
    
    def setup_routes(self):
        @self.app.get("/health")
        async def health():
            return {"status": "healthy", "model": self.model_name}
        
        @self.app.get("/")
        async def root():
            return {"message": "MLX-LM Server", "model": self.model_name}
        
        # Add catch-all for debugging
        @self.app.api_route("/{path:path}", methods=["GET", "POST", "PUT", "DELETE"])
        async def catch_all(path: str):
            print(f"Unhandled request to: /{path}", file=sys.stderr)
            return {"error": f"Endpoint /{path} not found", "available": ["/health", "/v1/chat/completions"]}
        
        @self.app.post("/v1/chat/completions")
        async def chat_completions(request: ChatCompletionRequest):
            print(f"Received chat completion request: {request.model}", file=sys.stderr)
            return await self.handle_completion(request)
        
        @self.app.post("/v1/responses")
        async def responses(request: ChatCompletionRequest):
            print(f"Received responses request: {request.model}", file=sys.stderr)
            return await self.handle_completion(request)
    
    async def handle_completion(self, request: ChatCompletionRequest):
        try:
            # Convert messages to prompt
            prompt = self.messages_to_prompt(request.messages)
            
            if request.stream:
                return StreamingResponse(
                    self.stream_generate(prompt, request),
                    media_type="text/plain"
                )
            else:
                response_text = await self.generate_response(prompt, request)
                
                # Create OpenAI-compatible response
                response = ChatCompletionResponse(
                    id=f"chatcmpl-{datetime.now().timestamp()}",
                    created=int(datetime.now().timestamp()),
                    model=request.model,
                    choices=[Choice(
                        index=0,
                        message=Message(role="assistant", content=response_text),
                        finish_reason="stop"
                    )],
                    usage=Usage(
                        prompt_tokens=len(prompt.split()),
                        completion_tokens=len(response_text.split()),
                        total_tokens=len(prompt.split()) + len(response_text.split())
                    )
                )
                
                return response
                
        except Exception as e:
            print(f"Error in completion: {e}", file=sys.stderr)
            raise HTTPException(status_code=500, detail=str(e))
    
    def messages_to_prompt(self, messages: List[Message]) -> str:
        """Convert OpenAI messages to a single prompt"""
        prompt = ""
        for message in messages:
            if message.role == "system":
                prompt += f"System: {message.content}\n\n"
            elif message.role == "user":
                prompt += f"User: {message.content}\n\n"
            elif message.role == "assistant":
                prompt += f"Assistant: {message.content}\n\n"
        
        prompt += "Assistant: "
        return prompt
    
    async def generate_response(self, prompt: str, request: ChatCompletionRequest) -> str:
        """Generate a single response"""
        if not MLX_LM_AVAILABLE or self.model is None:
            return f"Mock MLX response to: {prompt[:50]}..."
        
        try:
            response = generate(
                self.model,
                self.tokenizer,
                prompt,
                max_tokens=request.max_tokens or 512,
                temp=request.temperature or 0.7,
                top_p=request.top_p or 0.9,
                verbose=False
            )
            return response
        except Exception as e:
            print(f"MLX generation error: {e}", file=sys.stderr)
            return f"Error: {e}"
    
    async def stream_generate(self, prompt: str, request: ChatCompletionRequest):
        """Generate streaming response"""
        if not MLX_LM_AVAILABLE or self.model is None:
            # Mock streaming
            mock_response = f"Mock streaming MLX response to: {prompt[:50]}..."
            for i, word in enumerate(mock_response.split()):
                chunk = StreamResponse(
                    id=f"chatcmpl-{datetime.now().timestamp()}",
                    created=int(datetime.now().timestamp()),
                    model=request.model,
                    choices=[StreamChoice(
                        index=0,
                        delta={"content": word + " "},
                        finish_reason=None if i < len(mock_response.split()) - 1 else "stop"
                    )]
                )
                yield f"data: {chunk.json()}\n\n"
            yield "data: [DONE]\n\n"
            return
        
        try:
            for response in stream_generate(
                self.model,
                self.tokenizer,
                prompt,
                max_tokens=request.max_tokens or 512,
                temp=request.temperature or 0.7,
                top_p=request.top_p or 0.9
            ):
                chunk = StreamResponse(
                    id=f"chatcmpl-{datetime.now().timestamp()}",
                    created=int(datetime.now().timestamp()),
                    model=request.model,
                    choices=[StreamChoice(
                        index=0,
                        delta={"content": response.text},
                        finish_reason=None
                    )]
                )
                yield f"data: {chunk.json()}\n\n"
            
            # Send final chunk
            final_chunk = StreamResponse(
                id=f"chatcmpl-{datetime.now().timestamp()}",
                created=int(datetime.now().timestamp()),
                model=request.model,
                choices=[StreamChoice(
                    index=0,
                    delta={},
                    finish_reason="stop"
                )]
            )
            yield f"data: {final_chunk.json()}\n\n"
            yield "data: [DONE]\n\n"
            
        except Exception as e:
            print(f"MLX streaming error: {e}", file=sys.stderr)
            error_chunk = StreamResponse(
                id=f"chatcmpl-{datetime.now().timestamp()}",
                created=int(datetime.now().timestamp()),
                model=request.model,
                choices=[StreamChoice(
                    index=0,
                    delta={"content": f"Error: {e}"},
                    finish_reason="stop"
                )]
            )
            yield f"data: {error_chunk.json()}\n\n"
            yield "data: [DONE]\n\n"
    
    async def start_server(self):
        """Start the MLX server"""
        if not FASTAPI_AVAILABLE:
            print("Error: FastAPI not available. Install with: pip install fastapi uvicorn", file=sys.stderr)
            return
        
        print(f"Starting MLX-LM server on {self.host}:{self.port}")
        self.load_model()
        
        config = uvicorn.Config(
            self.app,
            host=self.host,
            port=self.port,
            log_level="info"
        )
        server = uvicorn.Server(config)
        await server.serve()


def main():
    parser = argparse.ArgumentParser(description='MLX-LM HTTP server for forge-rs')
    parser.add_argument('--model', required=True, help='MLX model name or path')
    parser.add_argument('--host', default='127.0.0.1', help='Server host (default: 127.0.0.1)')
    parser.add_argument('--port', type=int, default=8000, help='Server port (default: 8000)')
    
    args = parser.parse_args()
    
    try:
        server = MLXServer(args.model, args.host, args.port)
        asyncio.run(server.start_server())
    except KeyboardInterrupt:
        print("\nServer stopped by user")
    except Exception as e:
        print(f"Server error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()