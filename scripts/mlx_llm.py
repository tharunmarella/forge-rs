#!/usr/bin/env python3
"""
MLX-LM wrapper script for forge-rs
Provides local language model text generation using MLX on Apple Silicon
"""

import sys
import json
import argparse
import traceback
from typing import List, Dict, Any, Optional
from pathlib import Path

try:
    import mlx.core as mx
    from mlx_lm import load, generate, stream_generate
    from mlx_lm.utils import load_config
    MLX_LM_AVAILABLE = True
except ImportError as e:
    MLX_LM_AVAILABLE = False
    MLX_LM_IMPORT_ERROR = str(e)
    # Mock classes for testing
    class MockModel:
        pass
    
    class MockTokenizer:
        pass
    
    def mock_generate(model, tokenizer, prompt, **kwargs):
        return f"Mock response to: {prompt}"
    
    def mock_load(model_path, **kwargs):
        return MockModel(), MockTokenizer()


class MLXLanguageModel:
    """MLX-based language model for text generation"""
    
    def __init__(self, model_name: str, **kwargs):
        self.model_name = model_name
        self.model = None
        self.tokenizer = None
        self.config = kwargs
        
    def load_model(self):
        """Load the MLX language model"""
        if not MLX_LM_AVAILABLE:
            print(f"Warning: MLX-LM not available ({MLX_LM_IMPORT_ERROR}), using mock responses", file=sys.stderr)
            self.model = MockModel()
            self.tokenizer = MockTokenizer()
            return True
            
        try:
            # Load model and tokenizer
            self.model, self.tokenizer = load(self.model_name)
            return True
        except Exception as e:
            raise RuntimeError(f"Failed to load MLX-LM model '{self.model_name}': {e}")
    
    def generate_text(self, prompt: str, **kwargs) -> str:
        """Generate text completion for a prompt"""
        if self.model is None:
            self.load_model()
            
        if not MLX_LM_AVAILABLE:
            # Mock response for testing
            return f"Mock MLX-LM response to: {prompt[:50]}..."
            
        try:
            # Set default generation parameters
            generation_kwargs = {
                'max_tokens': kwargs.get('max_tokens', 512),
                'temp': kwargs.get('temperature', 0.7),
                'top_p': kwargs.get('top_p', 0.9),
                'verbose': False,
            }
            
            # Generate response
            response = generate(
                self.model,
                self.tokenizer, 
                prompt,
                **generation_kwargs
            )
            
            return response
            
        except Exception as e:
            print(f"Warning: MLX-LM generation failed: {e}", file=sys.stderr)
            return f"Error generating response: {e}"
    
    def stream_generate_text(self, prompt: str, **kwargs):
        """Stream text generation for a prompt"""
        if self.model is None:
            self.load_model()
            
        if not MLX_LM_AVAILABLE:
            # Mock streaming response
            mock_response = f"Mock streaming response to: {prompt[:50]}..."
            for word in mock_response.split():
                yield word + " "
            return
            
        try:
            # Set default generation parameters
            generation_kwargs = {
                'max_tokens': kwargs.get('max_tokens', 512),
                'temp': kwargs.get('temperature', 0.7),
                'top_p': kwargs.get('top_p', 0.9),
            }
            
            # Stream generation
            for response in stream_generate(
                self.model,
                self.tokenizer,
                prompt,
                **generation_kwargs
            ):
                yield response.text
                
        except Exception as e:
            print(f"Warning: MLX-LM streaming failed: {e}", file=sys.stderr)
            yield f"Error in streaming: {e}"


def main():
    parser = argparse.ArgumentParser(description='MLX-LM wrapper for forge-rs')
    parser.add_argument('--model', required=True, help='MLX model name or path')
    parser.add_argument('--prompt', help='Text prompt for generation')
    parser.add_argument('--input', help='Input JSON file with prompt and parameters')
    parser.add_argument('--output', help='Output JSON file (default: stdout)')
    parser.add_argument('--max-tokens', type=int, default=512, help='Maximum tokens to generate')
    parser.add_argument('--temperature', type=float, default=0.7, help='Generation temperature')
    parser.add_argument('--top-p', type=float, default=0.9, help='Top-p sampling')
    parser.add_argument('--stream', action='store_true', help='Enable streaming output')
    
    args = parser.parse_args()
    
    try:
        # Initialize language model
        llm = MLXLanguageModel(args.model)
        
        # Get prompt and parameters
        if args.input:
            with open(args.input, 'r') as f:
                data = json.load(f)
            prompt = data.get('prompt', '')
            generation_params = data.get('parameters', {})
        else:
            prompt = args.prompt or "Hello, how can I help you with coding?"
            generation_params = {
                'max_tokens': args.max_tokens,
                'temperature': args.temperature,
                'top_p': args.top_p,
            }
        
        if not prompt:
            raise ValueError("No prompt provided")
        
        # Generate response
        if args.stream:
            # Streaming mode
            result = {
                'model': args.model,
                'prompt': prompt,
                'streaming': True,
                'responses': []
            }
            
            for chunk in llm.stream_generate_text(prompt, **generation_params):
                result['responses'].append(chunk)
                if not args.output:
                    # Print streaming chunks to stderr for real-time feedback
                    print(chunk, end='', flush=True, file=sys.stderr)
            
            if not args.output:
                print()  # New line after streaming
        else:
            # Non-streaming mode
            response = llm.generate_text(prompt, **generation_params)
            result = {
                'model': args.model,
                'prompt': prompt,
                'response': response,
                'streaming': False,
                'parameters': generation_params
            }
        
        # Write output
        if args.output:
            with open(args.output, 'w') as f:
                json.dump(result, f, indent=2)
        else:
            json.dump(result, sys.stdout, indent=2)
            
    except Exception as e:
        error_result = {
            'error': str(e),
            'model': args.model,
            'traceback': traceback.format_exc() if '--debug' in sys.argv else None
        }
        
        if args.output:
            with open(args.output, 'w') as f:
                json.dump(error_result, f, indent=2)
        else:
            json.dump(error_result, sys.stdout, indent=2)
        
        sys.exit(1)


if __name__ == '__main__':
    main()