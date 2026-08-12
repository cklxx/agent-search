#!/usr/bin/env python3
"""Quantize reranker ONNX models to INT8 (dynamic quantization).

Usage:
    python3 scripts/quantize.py v2      # jina-reranker-v2 (fastembed path)
    python3 scripts/quantize.py v3      # jina-reranker-v3 (direct ort path)
    python3 scripts/quantize.py all

Dynamic quantization converts MatMul weights to INT8; activations stay
fp32 and are quantized at runtime. Expect ~4x smaller models and 2-3x
faster CPU inference with <1% NDCG/MRR regression.
"""

import shutil
import sys
from pathlib import Path

from onnxruntime.quantization import QuantType, quantize_dynamic

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"

V2_SRC = (
    MODELS
    / "models--jinaai--jina-reranker-v2-base-multilingual/snapshots"
)
V2_DST = MODELS / "jina-reranker-v2-int8"

V3_SRC = MODELS / "jina-reranker-v3/model.onnx"
V3_DST = MODELS / "jina-reranker-v3/model.int8.onnx"

TOKENIZER_FILES = [
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "config.json",
]


def quantize_v2() -> None:
    snapshots = sorted(V2_SRC.glob("*/onnx/model.onnx"))
    if not snapshots:
        sys.exit(f"v2 model not found under {V2_SRC}")
    # Keep the symlink path for snapshot_dir; reads follow the symlink.
    src = snapshots[-1]
    snapshot_dir = src.parents[1]

    V2_DST.mkdir(parents=True, exist_ok=True)
    dst = V2_DST / "model.onnx"
    print(f"v2: {src} ({src.stat().st_size >> 20} MiB) -> {dst}")
    quantize_dynamic(
        model_input=str(src),
        model_output=str(dst),
        weight_type=QuantType.QUInt8,
    )
    for name in TOKENIZER_FILES:
        f = snapshot_dir / name
        if f.exists():
            shutil.copy(f, V2_DST / name)
    print(f"v2: done, {dst.stat().st_size >> 20} MiB")


def quantize_v3() -> None:
    if not V3_SRC.exists():
        sys.exit(f"v3 model not found: {V3_SRC}")
    print(f"v3: {V3_SRC} -> {V3_DST}")
    # The model uses external data (2.2 GiB weights); the quantized output
    # also exceeds the 2 GiB protobuf limit, so keep external data format.
    quantize_dynamic(
        model_input=str(V3_SRC),
        model_output=str(V3_DST),
        weight_type=QuantType.QUInt8,
        use_external_data_format=True,
    )
    print(f"v3: done, {V3_DST.stat().st_size >> 20} MiB graph")


if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "all"
    if target in ("v2", "all"):
        quantize_v2()
    if target in ("v3", "all"):
        quantize_v3()
    if target not in ("v2", "v3", "all"):
        sys.exit(f"unknown target: {target} (v2|v3|all)")
