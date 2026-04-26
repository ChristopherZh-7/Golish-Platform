export interface DiffLine {
  type: "addition" | "deletion" | "context";
  content: string;
  lineNumber?: number;
}
