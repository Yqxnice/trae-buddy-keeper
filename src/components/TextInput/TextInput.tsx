import type { InputHTMLAttributes, TextareaHTMLAttributes } from "react";
import "./TextInput.css";

/** 单行输入框 */
export function TextInput({ className = "", ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={`inp ${className}`} {...rest} />;
}

/** 数字小输入框（定时参数用，居中窄宽） */
export function NumInput({ className = "", ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input type="number" className={`inp num ${className}`} {...rest} />;
}

/** 多行输入框（凭据 / JSON 导入用） */
export function TextArea({ className = "", ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={`inp area ${className}`} {...rest} />;
}
