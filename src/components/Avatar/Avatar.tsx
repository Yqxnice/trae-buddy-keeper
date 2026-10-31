import { useState } from "react";
import "./Avatar.css";

interface AvatarProps {
  name: string;
  /** 登录链路抓到的头像 URL，加载失败回退首字母 */
  src?: string;
  /** sm 用于积分包卡片行头 */
  size?: "md" | "sm";
}

/** 圆形头像：有图显示图，无图/图挂则显示首字母圆标 */
export function Avatar({ name, src, size = "md" }: AvatarProps) {
  const [broken, setBroken] = useState(false);
  const showImg = src && !broken;
  return (
    <span className={`ava ${size === "sm" ? "sm" : ""}`}>
      {showImg ? (
        <img src={src} alt={name} onError={() => setBroken(true)} />
      ) : (
        (name || "?").charAt(0).toUpperCase()
      )}
    </span>
  );
}
