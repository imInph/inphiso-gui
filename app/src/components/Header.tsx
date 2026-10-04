import type { ReactNode } from "react";
import { LogoArrow } from "./icons";

export function Header({ right }: { right?: ReactNode }) {
  return (
    <header className="header">
      <div className="brand">
        <div className="brand-mark">
          <LogoArrow />
        </div>
        <div className="brand-name">inphiso</div>
      </div>
      {right}
    </header>
  );
}
