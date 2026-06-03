import torch
from safetensors.torch import save_file
import os, shutil

cache_dir = os.path.expanduser(r"~\.cache\huggingface\hub\models--microsoft--graphcodebert-base\blobs")
bin_path = os.path.join(cache_dir, "pytorch_model.bin")
out_dir = r"g:\code-memory\models\microsoft--graphcodebert-base"
os.makedirs(out_dir, exist_ok=True)

print(f"Loading: {bin_path}")
state_dict = torch.load(bin_path, map_location="cpu", weights_only=True)
print(f"Loaded {len(state_dict)} tensors")

# Clone tensors to break shared memory references
fixed_dict = {}
for k, v in state_dict.items():
    fixed_dict[k] = v.clone()

out_path = os.path.join(out_dir, "model.safetensors")
save_file(fixed_dict, out_path)
size_mb = os.path.getsize(out_path) / 1024 / 1024
print(f"Saved to: {out_path} ({size_mb:.1f} MB)")

# Copy config files
for f in ["config.json", "vocab.json", "merges.txt", "special_tokens_map.json", "tokenizer_config.json"]:
    src = os.path.join(cache_dir, f)
    if os.path.exists(src):
        shutil.copy2(src, os.path.join(out_dir, f))
        print(f"Copied: {f}")

print("Done!")