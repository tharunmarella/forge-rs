#!/usr/bin/env python3
"""
MLX-Embeddings wrapper script for forge-rs
Provides text embedding generation using MLX on Apple Silicon
"""

import sys
import json
import argparse
import traceback
from typing import List, Dict, Any
from pathlib import Path

try:
    import mlx.core as mx
    from mlx_embeddings import load
    from mlx_embeddings.utils import get_model_path
    MLX_AVAILABLE = True
except ImportError as e:
    MLX_AVAILABLE = False
    MLX_IMPORT_ERROR = str(e)
    # Mock MLX for testing purposes
    class MockMLX:
        @staticmethod
        def array(data):
            return data
        @staticmethod  
        def no_grad():
            return MockContext()
        @staticmethod
        def mean(data, axis=None):
            return MockTensor([0.1] * 768)  # Mock 768-dim embedding
    
    class MockContext:
        def __enter__(self):
            return self
        def __exit__(self, *args):
            pass
    
    class MockTensor:
        def __init__(self, data):
            self.data = data
        def squeeze(self):
            return self
        def tolist(self):
            return self.data
    
    mx = MockMLX()
    
    class MockModel:
        def __call__(self, input_ids):
            # Return mock embeddings
            return MockTensor([0.1] * 768)
    
    class MockTokenizer:
        def encode(self, text):
            # Return mock token IDs based on text length
            return list(range(min(len(text.split()), 512)))


class MLXEmbeddingGenerator:
    """MLX-based embedding generator"""
    
    def __init__(self, model_name: str):
        self.model_name = model_name
        self.model = None
        self.tokenizer = None
        
    def load_model(self):
        """Load the MLX embedding model"""
        if not MLX_AVAILABLE:
            print(f"Warning: MLX not available ({MLX_IMPORT_ERROR}), using mock embeddings", file=sys.stderr)
            # Create mock model and tokenizer for testing
            self.model = MockModel()
            self.tokenizer = MockTokenizer()
            return True
            
        try:
            # Load model and tokenizer
            self.model, self.tokenizer = load(self.model_name)
            return True
        except Exception as e:
            raise RuntimeError(f"Failed to load MLX model '{self.model_name}': {e}")
    
    def generate_embeddings(self, texts: List[str]) -> List[List[float]]:
        """Generate embeddings for a list of texts"""
        if self.model is None:
            self.load_model()
            
        embeddings = []
        
        for text in texts:
            try:
                # Tokenize the text
                tokens = self.tokenizer.encode(text)
                
                # Convert to MLX array
                input_ids = mx.array([tokens])
                
                # Generate embeddings
                with mx.no_grad():
                    outputs = self.model(input_ids)
                    
                # Extract embeddings (typically the last hidden state mean pooled)
                if hasattr(outputs, 'last_hidden_state'):
                    embedding = mx.mean(outputs.last_hidden_state, axis=1).squeeze()
                elif hasattr(outputs, 'pooler_output'):
                    embedding = outputs.pooler_output.squeeze()
                else:
                    # Fallback: use the output directly
                    embedding = outputs.squeeze()
                
                # Convert to Python list
                embedding_list = embedding.tolist()
                embeddings.append(embedding_list)
                
            except Exception as e:
                # Return zero vector on error
                print(f"Warning: Failed to embed text '{text[:50]}...': {e}", file=sys.stderr)
                embeddings.append([0.0] * 768)  # Default dimension
                
        return embeddings


def main():
    parser = argparse.ArgumentParser(description='MLX-Embeddings wrapper for forge-rs')
    parser.add_argument('--model', required=True, help='MLX model name or path')
    parser.add_argument('--input', help='Input JSON file with texts (default: stdin)')
    parser.add_argument('--output', help='Output JSON file (default: stdout)')
    parser.add_argument('--batch-size', type=int, default=32, help='Batch size for processing')
    
    args = parser.parse_args()
    
    try:
        # Read input texts
        if args.input:
            with open(args.input, 'r') as f:
                data = json.load(f)
        else:
            data = json.load(sys.stdin)
        
        texts = data.get('texts', [])
        if not texts:
            raise ValueError("No texts provided in input")
        
        # Initialize embedding generator
        generator = MLXEmbeddingGenerator(args.model)
        
        # Generate embeddings in batches
        all_embeddings = []
        batch_size = args.batch_size
        
        for i in range(0, len(texts), batch_size):
            batch_texts = texts[i:i + batch_size]
            batch_embeddings = generator.generate_embeddings(batch_texts)
            all_embeddings.extend(batch_embeddings)
        
        # Prepare output
        result = {
            'embeddings': all_embeddings,
            'model': args.model,
            'count': len(all_embeddings)
        }
        
        # Write output
        if args.output:
            with open(args.output, 'w') as f:
                json.dump(result, f)
        else:
            json.dump(result, sys.stdout)
            
    except Exception as e:
        error_result = {
            'error': str(e),
            'traceback': traceback.format_exc() if '--debug' in sys.argv else None
        }
        
        if args.output:
            with open(args.output, 'w') as f:
                json.dump(error_result, f)
        else:
            json.dump(error_result, sys.stdout)
        
        sys.exit(1)


if __name__ == '__main__':
    main()