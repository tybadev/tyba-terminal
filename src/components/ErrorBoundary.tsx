import { Component, type ErrorInfo, type ReactNode } from "react";
import { withTranslation, type WithTranslation } from "react-i18next";
import { Warning } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";

interface Props extends WithTranslation {
  region: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
}

class RegionErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[tyba] erro em ${this.props.region}`, error, info);
  }

  reset = () => this.setState({ error: null });

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    const { t, region } = this.props;
    return (
      <div
        role="alert"
        className="flex h-full w-full items-center justify-center p-4"
      >
        <div className="flex max-w-sm flex-col items-center gap-2 rounded-md border border-tyba-red/40 bg-tyba-red-tint px-4 py-3 text-center">
          <Warning size={18} weight="bold" className="text-tyba-red" />
          <p className="text-xs font-medium text-tyba-text">
            {t("regionCrashed", { region })}
          </p>
          <p className="font-mono text-[10px] break-all text-tyba-text-faint">
            {error.message}
          </p>
          <Button
            size="sm"
            variant="outline"
            onClick={this.reset}
            className="mt-1 h-6 rounded-[4px] px-2.5 text-[11px]"
          >
            {t("retry")}
          </Button>
        </div>
      </div>
    );
  }
}

export const ErrorBoundary = withTranslation()(RegionErrorBoundary);
