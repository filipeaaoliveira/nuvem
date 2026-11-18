// Expressive Code plugin: every code block is a captioned plate.
//
// - A block without a title is captioned with its language, so a reader can
//   tell a shell command from a config file at a glance.
// - Shell blocks show a `$` prompt before each command. The prompt is drawn
//   by CSS and is not part of the text, so selecting or copying the block
//   gives the command alone. A line that continues the previous one (after a
//   trailing backslash) gets no prompt.

const SHELL = new Set(['bash', 'sh', 'shell', 'zsh']);

const CAPTIONS = {
	bash: 'Shell',
	sh: 'Shell',
	shell: 'Shell',
	zsh: 'Shell',
	console: 'Terminal',
	toml: 'TOML',
	json: 'JSON',
	yaml: 'YAML',
	yml: 'YAML',
	rust: 'Rust',
	rs: 'Rust',
	diff: 'Diff',
	text: 'Text',
	txt: 'Text',
	plaintext: 'Text',
	'': 'Text',
};

export function pluginCodePlates() {
	return {
		name: 'nuvem-code-plates',
		hooks: {
			preprocessCode: ({ codeBlock }) => {
				if (codeBlock.props.title) return;
				const lang = (codeBlock.language ?? '').toLowerCase();
				codeBlock.props.title = CAPTIONS[lang] ?? lang.toUpperCase();
			},
			postprocessRenderedLine: ({ codeBlock, lineIndex, line, renderData }) => {
				if (!SHELL.has((codeBlock.language ?? '').toLowerCase())) return;
				if (line.text.trim() === '') return;
				const previous = lineIndex > 0 ? codeBlock.getLine(lineIndex - 1)?.text ?? '' : '';
				if (previous.trimEnd().endsWith('\\')) return;
				const props = renderData.lineAst.properties;
				const classes = Array.isArray(props.className) ? props.className : props.className ? [props.className] : [];
				props.className = [...classes, 'nv-command'];
			},
		},
		baseStyles: `
			.nv-command .code::before {
				content: '$ ';
				color: var(--nv-naples);
				user-select: none;
				-webkit-user-select: none;
			}
		`,
	};
}
