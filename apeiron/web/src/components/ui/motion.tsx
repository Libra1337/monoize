import * as React from "react";
import { motion, useReducedMotion, type Variants } from "framer-motion";
import { cn } from "@/lib/utils";

export const springs = {
  snappy: { type: "spring", stiffness: 500, damping: 30 },
  smooth: { type: "spring", stiffness: 300, damping: 30 },
  bouncy: { type: "spring", stiffness: 400, damping: 15 },
} as const;

export const easings = {
  easeOutExpo: [0.16, 1, 0.3, 1],
  easeOutQuart: [0.25, 1, 0.5, 1],
} as const;

const riseVariants: Variants = {
  hidden: { opacity: 0, y: 26, filter: "blur(10px)" },
  visible: { opacity: 1, y: 0, filter: "blur(0px)" },
};

export function PageWrapper({ className, children }: { className?: string; children: React.ReactNode }) {
  const reduced = useReducedMotion();
  return (
    <motion.div
      initial={reduced ? { opacity: 0 } : "hidden"}
      animate={reduced ? { opacity: 1 } : "visible"}
      transition={reduced ? { duration: 0.2 } : { duration: 0.7, ease: easings.easeOutQuart }}
      variants={riseVariants}
      className={cn("flex flex-1 flex-col", className)}
    >
      {children}
    </motion.div>
  );
}

export function ScrollReveal({
  className,
  children,
  delay = 0,
}: {
  className?: string;
  children: React.ReactNode;
  delay?: number;
}) {
  const reduced = useReducedMotion();
  return (
    <motion.div
      initial={reduced ? { opacity: 0 } : { opacity: 0, y: 24 }}
      whileInView={reduced ? { opacity: 1 } : { opacity: 1, y: 0 }}
      viewport={{ once: true, margin: "-64px" }}
      transition={reduced ? { duration: 0.2 } : { duration: 0.6, ease: easings.easeOutExpo, delay }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

export function SectionKicker({ index, label }: { index: string; label: string }) {
  return (
    <div className="flex items-center gap-3 font-mono text-sm text-primary">
      <motion.span
        initial={{ scaleX: 0 }}
        whileInView={{ scaleX: 1 }}
        viewport={{ once: true }}
        transition={{ duration: 0.6, ease: easings.easeOutExpo }}
        className="h-px w-7 origin-left bg-primary/70"
      />
      <span>
        {index} · {label}
      </span>
    </div>
  );
}
