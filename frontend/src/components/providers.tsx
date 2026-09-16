"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import { LazyMotion, domAnimation } from "framer-motion";
import { useEffect, useState } from "react";
import { Toaster } from "sonner";
import { MotionProvider } from "@/lib/motion/MotionProvider";
import { clearThemeTransitionArtifacts } from "@/lib/motion/theme-spread";
import { makeQueryClient } from "@/lib/query-client";

const THEME_STORAGE_KEY = "rav-theme";

function ThemeInitializer() {
  useEffect(() => {
    clearThemeTransitionArtifacts();
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "dark") {
      document.documentElement.classList.add("dark");
    } else if (stored === "light") {
      document.documentElement.classList.remove("dark");
    } else {
      // Dark is the altacee default, not an opt-in — a first visit gets the
      // brand's own state regardless of the OS preference. The toggle still
      // wins once the viewer has chosen, because that writes the stored key.
      document.documentElement.classList.add("dark");
    }
  }, []);

  return null;
}

export function Providers({ children }: { children: React.ReactNode }) {
  const [queryClient] = useState(() => makeQueryClient());
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeInitializer />
      <LazyMotion features={domAnimation}>
        <MotionProvider>{children}</MotionProvider>
      </LazyMotion>
      <Toaster
        position="bottom-center"
        toastOptions={{
          className:
            "!bg-foreground !text-background !border-border !shadow-lg !rounded-lg !text-sm",
        }}
      />
    </QueryClientProvider>
  );
}
