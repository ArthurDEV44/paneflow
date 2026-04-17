"use client";

import { FadeIn } from "./fade-in";
import { Copy, Check } from "lucide-react";
import { useState } from "react";

export function Install() {
  return (
    <section className="py-24 sm:py-32 border-t border-surface-border">
      <div className="max-w-3xl mx-auto px-6">
        <FadeIn>
          <div className="text-center mb-12">
            <h2 className="text-3xl sm:text-4xl font-bold tracking-tight mb-4">
              Get started
            </h2>
            <p className="text-text-muted">
              Build from source with the Rust toolchain. Linux only.
            </p>
          </div>

          <div className="code-block relative">
            <CopyButton
              text={`git clone https://github.com/ArthurDEV44/paneflow\ncd paneflow && cargo build --release\n./target/release/paneflow-app`}
            />
            <div>
              <span className="comment"># Clone and build</span>
            </div>
            <div>
              <span className="command">git clone</span>{" "}
              <span className="text-text">
                https://github.com/ArthurDEV44/paneflow
              </span>
            </div>
            <div>
              <span className="command">cd</span>{" "}
              <span className="text-text">paneflow</span>{" "}
              <span className="text-text-muted">&&</span>{" "}
              <span className="command">cargo build</span>{" "}
              <span className="flag">--release</span>
            </div>
            <div className="mt-3">
              <span className="comment"># Run</span>
            </div>
            <div>
              <span className="command">./target/release/paneflow-app</span>
            </div>
          </div>

          <div className="mt-6 text-center text-sm text-text-subtle">
            Requires Rust 1.82+, Vulkan-capable GPU, and Linux (Wayland or X11)
          </div>
        </FadeIn>
      </div>
    </section>
  );
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <button
      onClick={handleCopy}
      className="absolute top-4 right-4 p-2 rounded-md border border-surface-border hover:border-surface-border-hover text-text-subtle hover:text-text-muted transition-all duration-200"
      aria-label="Copy to clipboard"
    >
      {copied ? (
        <Check className="w-4 h-4 text-accent-green" />
      ) : (
        <Copy className="w-4 h-4" />
      )}
    </button>
  );
}
