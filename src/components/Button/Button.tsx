import type { ButtonHTMLAttributes } from "react";
import "./Button.css";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** pri 品牌实心 / ghost 透明 / danger 危险红 / 默认描边 */
  variant?: "default" | "pri" | "ghost" | "danger";
}

/** Fluent 风格按钮：32px 高、8px 圆角，按下轻微下沉 */
export function Button({ variant = "default", className = "", ...rest }: ButtonProps) {
  const v = variant === "default" ? "" : ` ${variant}`;
  return <button className={`btn${v} ${className}`} {...rest} />;
}
