import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "PXE Network Control",
  description: "Operations console for PXE, TFTP, and HTTP netboot infrastructure",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
