import { Eye, EyeOff } from "lucide-react";
import { useState } from "react";
import { Input } from "@/components/ui/input";

export function PasswordInput({
  id,
  value,
  onChange,
  placeholder,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
}) {
  const [showPassword, setShowPassword] = useState(false);

  return (
    <div className="relative group">
      <Input
        id={id}
        type={showPassword ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="pr-10 font-mono text-[12px] bg-foreground/[0.03] border-foreground/[0.06] focus:border-accent/40 focus:ring-accent/10 transition-colors"
      />
      <button
        type="button"
        onClick={() => setShowPassword(!showPassword)}
        className="absolute right-2 top-1/2 -translate-y-1/2 p-1 rounded text-muted-foreground/40 hover:text-foreground/80 hover:bg-foreground/[0.05] transition-all"
      >
        {showPassword ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
      </button>
    </div>
  );
}
