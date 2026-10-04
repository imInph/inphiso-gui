// Stroke icons from the mockups (24×24 grid, round caps).
import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement> & { size?: number; weight?: number };

function Icon({ size = 16, weight = 2, children, ...rest }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={weight}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const LogoArrow = (p: IconProps) => (
  <Icon size={14} weight={2.6} {...p}>
    <path d="M12 3v12M6 10l6 6 6-6M5 21h14" />
  </Icon>
);

export const Sliders = (p: IconProps) => (
  <Icon size={20} weight={1.8} {...p}>
    <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
    <circle cx="16" cy="7" r="2" />
    <circle cx="8" cy="17" r="2" />
  </Icon>
);

export const Disc = (p: IconProps) => (
  <Icon size={32} weight={1.6} {...p}>
    <circle cx="12" cy="12" r="9" />
    <circle cx="12" cy="12" r="2.5" />
  </Icon>
);

export const Check = (p: IconProps) => (
  <Icon weight={2.4} {...p}>
    <path d="M5 12.5 10 17.5 19 7" />
  </Icon>
);

export const ArrowRight = (p: IconProps) => (
  <Icon weight={2.4} {...p}>
    <path d="M5 12h14M13 6l6 6-6 6" />
  </Icon>
);

export const Cross = (p: IconProps) => (
  <Icon weight={2.4} {...p}>
    <path d="M6 6l12 12M18 6 6 18" />
  </Icon>
);

export const Alert = (p: IconProps) => (
  <Icon weight={2.2} {...p}>
    <path d="M12 8v5M12 16.5v.01" />
    <circle cx="12" cy="12" r="9" />
  </Icon>
);

export const Download = (p: IconProps) => (
  <Icon size={32} weight={1.6} {...p}>
    <path d="M12 4v11M7 10.5l5 5 5-5M5 20h14" />
  </Icon>
);

export const Windows = (p: IconProps) => (
  <Icon size={32} weight={1.6} {...p}>
    <path d="M4 5.5 11 4.5v7H4zM13 4.2 20 3v8.5h-7zM4 12.5h7v7l-7-1zM13 12.5h7V21l-7-1.2z" />
  </Icon>
);
