import type { ReactNode } from "react";
import "./PageHeader.css";

/** 页头：标题 + 副标题，右侧可挂操作按钮 */
export function PageHeader({
  title,
  sub,
  right,
}: {
  title: string;
  sub?: ReactNode;
  right?: ReactNode;
}) {
  return (
    <div className="vh">
      <div>
        <h2>{title}</h2>
        {sub && <div className="sub">{sub}</div>}
      </div>
      {right && <div className="vh-right">{right}</div>}
    </div>
  );
}
