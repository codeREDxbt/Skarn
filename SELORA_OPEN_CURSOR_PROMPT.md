# تطوير Selora IDE بالاستفادة من OpenCursor

## 1. أساس المقارنة

هذه مقارنة مبنية على قراءة المصدر، وليست مقارنة تسويقية أو نتيجة اختبار تشغيل للتطبيقين.

- المصدر: <https://github.com/noah-thing/open-cursor>
- نسخة OpenCursor المفحوصة: `2ac2b8fd7dde25d6c574a109913c874f9c3b959c`.
- [مجلد الإضافة عند النسخة المفحوصة](https://github.com/noah-thing/open-cursor/tree/2ac2b8fd7dde25d6c574a109913c874f9c3b959c/extensions/open-cursor).
- نسخة Selora الأساسية: `91618cbdfa196620eaba2829dbae5a1a8d1c728c`، مع قراءة التعديلات المحلية والملفات الجديدة الموجودة وقت الفحص.
- جُلب المصدر باستخدام shallow sparse clone؛ المجلدات المتاحة محليًا هي `extensions/open-cursor` و`docs` والملفات الجذرية، وليس checkout كاملًا لكل Code OSS.
- مكان المرجع المحلي: `C:\Users\Toxic\AppData\Local\Temp\opencode\open-cursor-reference-20261101`.
- جميع مسارات OpenCursor أدناه نسبية إلى جذر هذا المرجع.

الفرق المعماري مهم: OpenCursor مبني على Code OSS وواجهات VS Code. تطبيق Selora الحالي مبني على Tauri 2 وRust وReact وCodeMirror 6 وxterm. نستفيد من الخوارزميات والعقود وتجربة الاستخدام، ثم نكيّف التكامل مع Selora.

عبارة «غير موجود» أدناه تعني أن مسار الميزة لم يظهر في كود تطبيق سطح المكتب الذي فُحص. يجب إعادة التحقق قبل التنفيذ لأن المشروع يتغير.

## 2. ما الموجود فعلًا في Selora؟

لا تبدأ بإعادة بناء هذه المزايا:

- جلسات وكيل متعددة، حفظ المحادثات، خطط، قوائم مهام، موافقات، وإلغاء الطلبات.
- Subagents وWorkflows وتتبع نتائجها؛ توجد أيضًا أعمال محلية جديدة لنافذة Agents مستقلة ومسودات محادثة.
- محرر CodeMirror مع LSP للإكمال التقليدي وhover والتعريفات والتشخيصات.
- Terminal مدمج، Git، diffs موحّدة ومقسّمة، وإدارة الفروع وعمليات commit.
- إرفاق مسارات ملفات المشروع، وإرسال بيانات الملف النشط والتحديد وحالة dirty.
- مزوّدات API، إعداد Ollama، اختيار النموذج، وإعداد reasoning effort.
- منطق ghost completion في Rust والواجهة القديمة: `crates/selora-ai/src/client.rs:570` و`crates/selora-editor/src/ghost_text.rs` و`crates/selora-app/src/shell.rs`.

هناك فروق يجب عدم الخلط بينها:

1. `CodeMirrorPane.tsx:539` يربط إكمال LSP؛ لم يظهر ربط ghost completion الذكي في محرر سطح المكتب الحالي.
2. `features/editor/pending-edits.ts` يتتبع مزامنة تعديلات المستخدم مع backend. ليس نظام مراجعة تعديلات AI، رغم تشابه الاسم مع OpenCursor.
3. `ToolCard.tsx` يعرض diff وموافقة للعملية. هذا لا يساوي الاحتفاظ/التراجع لكل hunk بعد التعديل.
4. `sessions.rs:2468` يعيد المحادثة قبل رسالة لإعادة المحاولة، ويرفض المحاولات ذات side effects عند `:2544`. ليس استعادة لملفات المشروع.
5. checkpoints الخاصة بـWorkflows تحفظ تاريخ الوكيل للاستئناف؛ لا تثبت وجود snapshots لملفات كل رسالة.
6. `AgentComposer.tsx:148` يرفق مسارات فقط. `EditorContextDto` في `crates/selora-protocol/src/lib.rs:222` لا يحمل محتوى الملف غير المحفوظ.
7. Ask في واجهة Selora يعني طلب الموافقة قبل التغيير. Ask في OpenCursor وضع قراءة فقط. لا تنقل الاسم وتغيّر سلوكه ضمنيًا.
8. `apps/selora-desktop/frontend/src/features/agents-workspace/AgentsChangesView.tsx` يعرض Git changes بالفعل. لا تنشئ لوحة ثانية بالاسم نفسه، ولا تفترض أن هذه اللوحة تتتبع ملكية تعديلات AI.

## 3. الميزات المقترحة مرتبة بالأولوية

### الأولوية الأولى: تحسين التحرير اليومي

**A. إكمال AI فعلي داخل CodeMirror** — موجود جزئيًا في Rust، والناقص تكامل سطح المكتب وتحسينه.

اقتراحات متعددة الأسطر، Tab للقبول، Escape للإخفاء، نموذج سريع مستقل، وسياق قبل المؤشر وبعده. أهم أفكار المصدر: إلغاء الطلب القديم، cache، منع تكرار النص اللاحق، والمحافظة على indentation.

**B. Ctrl+K للتعديل داخل الملف** — لم يظهر مسار مماثل في التطبيق الحالي.

صندوق صغير بجوار التحديد، كتابة طلب، عرض diff متدرج، ثم Accept / Reject / Retry. عند عدم وجود تحديد يُنشأ نص عند المؤشر. لا يحتاج المستخدم إلى الانتقال إلى المحادثة.

**C. مراجعة تغييرات الوكيل لكل hunk وملف** — توسعة للموافقات وdiff الحاليين.

قائمة Changes مرتبطة بالدور الحالي، مع Keep / Undo لكل جزء، لكل ملف، أو لكل مجموعة تغييرات. لا تعِد بناء Git diff؛ هذه مراجعة تعديلات AI مستقلة عن stage وcommit.

**D. استعادة ملفات المشروع إلى checkpoint رسالة** — استعادة المحادثة موجودة جزئيًا؛ استعادة الملفات ناقصة.

التقاط المحتوى قبل أول تعديل لكل ملف في الدور، ومعالجة الإنشاء والحذف، واستعادة لاحقة تراعي التعديلات اليدوية وتعارض الجلسات.

### الأولوية الثانية: سياق أفضل وقدرات قابلة للتوسعة

**E. قائمة @ ومرفقات typed** — توسعة لإرفاق الملفات الموجود.

ملفات، مجلدات، تحديد المحرر، رموز، Git diff، Problems، Terminal، روابط توثيق، محادثات سابقة، وصور للنماذج الداعمة.

**F. قواعد المشروع وSlash Commands** — لم يظهر loader لهذه الميزة في المسارات المفحوصة.

`AGENTS.md` و`CLAUDE.md` وقواعد `.selora/rules` وأوامر `.selora/commands`، مع نطاقات glob وإظهار القواعد التي استُخدمت بالفعل.

**G. MCP داخل Selora** — لم يظهر client manager أو تكامل أدوات MCP في مسار الوكيل الحالي.

إعدادات servers، اتصال stdio وHTTP، عرض الحالة والأدوات، وربطها بموجّه الأدوات والموافقات الموجودين.

**H. بحث دلالي في المشروع** — البحث النصي موجود؛ embedding index لم يظهر.

فهرسة تدريجية مع استبعاد الملفات غير المناسبة، نتائج بمسارات وأسطر، وعودة واضحة للبحث النصي عند عدم وجود embeddings.

### الأولوية الثالثة: راحة الاستخدام والتكامل

**I. Ctrl+K داخل Terminal** — اقتراح أمر مناسب لـPowerShell من وصف طبيعي، مع معرفة shell وcwd والمخرجات المحددة.

**J. تشغيل Claude Code وCodex كخلفيات للوكيل** — تكامل CLI لم يظهر في مسار Selora الحالي.

الاستفادة من CLI المثبت والمسجّل دخوله، مع توحيد أحداث streaming والأدوات والإلغاء والجلسات داخل واجهة Selora.

**K. إعداد نماذج محلية موجه للمستخدم** — Ollama موجود؛ الناقص الاكتشاف والإرشاد والتنزيل وحالة التقدم.

**L. توليد رسالة commit** — إدارة Git موجودة؛ لم يظهر زر لتوليد الرسالة من diff.

وضع Debug المنظم والمساعدة الداخلية أفكار لاحقة مفيدة. لا تبدأ بها قبل اكتمال مسارات التحرير والمراجعة والسياق.

## 4. خريطة الأكواد المناسبة للاقتباس

هذه خريطة إعادة استخدام، وليست ادعاءً بأن نقل الملف كما هو سيعمل في Selora.

| الميزة | مرجع OpenCursor | ما نستفيد منه | التكييف المطلوب |
|---|---|---|---|
| AI Tab | `extensions/open-cursor/src/tab/logic.ts` | `extractWindow`, `postProcess`, `trimOverlapInfo`, `CompletionCache` | منطق TypeScript قابل للاستخراج؛ اربطه بمستند CodeMirror وإصداره |
| AI Tab lifecycle | `extensions/open-cursor/src/tab/provider.ts` | debounce، الإلغاء، اختيار FIM أو chat، snippets | أعد كتابة adapter؛ هذا الملف يعتمد على `vscode` |
| Inline edit | `extensions/open-cursor/src/inline/region.ts` و`extensions/open-cursor/src/inline/fence.ts` | `computeRegion`, `buildInlinePrompt`, تحليل streaming fences | حدّد offsets وCRLF وUTF-16 بدقة مع بروتوكول Selora |
| Inline lifecycle | `extensions/open-cursor/src/inline/inlineEdit.ts` | جلسة تعديل، follow-up، دفعات عرض كل 100ms، معالجة تغير المنطقة | أنشئ CodeMirror extension وخدمة Rust بدل أوامر VS Code |
| Review | `extensions/open-cursor/src/edits/hunks.ts` | `computeHunks`, `foldAccept`, `applyReject`, `minimalRawEdit` | الملف يعتمد على حزمة `diff`؛ قرّر المكتبة أو port إلى Rust بعد مراجعة الاعتماد |
| Checkpoints | `extensions/open-cursor/src/edits/checkpointLogic.ts` و`extensions/open-cursor/src/edits/checkpoints.ts` | `planRestore` والتقاط أول before-content وcontent-addressed blobs | التخزين وعمليات الملفات في Rust مع معالجة التعارضات والاسترداد |
| Context | `extensions/open-cursor/src/context/mentions.ts` و`extensions/open-cursor/src/shared/protocol.ts` | أنواع السياق، بحث الملفات، hooks للمحادثات، حدود المحتوى | DTOs جديدة ومصادر سياق من backend بدل VS Code API |
| صور | `extensions/open-cursor/webview/src/chat/composer.tsx` | paste/drop، preview، attachment chips | افحص دعم multimodal في مزوّد Selora؛ لا تكتفِ بعرض الصورة |
| Rules | `extensions/open-cursor/src/context/rules.ts` و`extensions/open-cursor/src/context/frontmatter.ts` | parsing، globs، ترتيب المصادر، cache invalidation | loader في Rust ومسارات `.selora` |
| MCP | `extensions/open-cursor/src/mcp/config.ts` و`extensions/open-cursor/src/mcp/manager.ts` | دمج الإعدادات، namespaced tools، lazy connection وhot reload | استخدم client مناسبًا لـRust؛ لا تضف Node runtime لمجرد نسخ الملف |
| Indexing | `extensions/open-cursor/src/indexing/chunker.ts` و`extensions/open-cursor/src/indexing/ignore.ts` و`extensions/open-cursor/src/indexing/vectorStore.ts` و`extensions/open-cursor/src/indexing/index.ts` | chunking، الاستبعاد، حفظ vectors، تحديث تدريجي وkeyword fallback | خدمة خلفية في Rust وحدود CPU وذاكرة وI/O |
| Terminal AI | `extensions/open-cursor/src/terminal/logic.ts` و`extensions/open-cursor/src/terminal/cmdK.ts` | `OutputBuffer`, `buildTerminalPrompt`, command preview | استخدم xterm وConPTY، ولا تنسخ منطق تنظيف الأوامر دون اختبارات PowerShell |
| CLI agents | `extensions/open-cursor/src/cliAgents/types.ts` و`extensions/open-cursor/src/cliAgents/claude.ts` و`extensions/open-cursor/src/cliAgents/codex.ts` | lifecycle، parsing الأحداث، resume، cancellation | إدارة subprocess في Rust وتوحيد الأحداث مع بروتوكول Selora |
| Local setup | `extensions/open-cursor/src/connect/localModels.ts` و`extensions/open-cursor/src/connect/localSetup.ts` و`extensions/open-cursor/src/connect/lmStudio.ts` | ملاءمة الذاكرة، اكتشاف الخدمة، تقدم الإعداد | معلومات العتاد عبر Rust؛ تحقق من النموذج فعليًا بدل نسخ catalog ثابت |
| Commit message | `extensions/open-cursor/src/scm/logic.ts` و`extensions/open-cursor/src/scm/commitMessage.ts` | prompt، تنظيف الإجابة، مراعاة أسلوب commits | استخدم `selora-git` وواجهة Git الحالية |
| Model routing | `extensions/open-cursor/src/providers/registry.ts` | فصل نموذج chat عن inline وTab | وسّع إعدادات Selora وقدرات النماذج بدل إنشاء registry موازٍ كامل |

مرجع الترخيص: [LICENSE.txt عند النسخة المفحوصة](https://github.com/noah-thing/open-cursor/blob/2ac2b8fd7dde25d6c574a109913c874f9c3b959c/LICENSE.txt). الترخيص MIT ويذكر Noah وMicrosoft. عند نسخ كود أو جزء جوهري احتفظ بإشعار الحقوق والترخيص وسجّل المصدر والنسخة. الاقتراحات الإضافية أدناه، مثل قياسات الأداء والتعارضات متعددة الجلسات، متطلبات لـSelora وليست مزايا مثبتة في المصدر المرجعي.

---

## 5. البرومبت الجاهز للتنفيذ

أرفق هذا الملف كاملًا للعميل المنفّذ، واطلب منه تنفيذ القسم الخامس. الأقسام السابقة جزء من مرجعه، خصوصًا جدول المسارات. عند النسخ اليدوي، انسخ القسم الخامس ومعه جدول القسم الرابع.

### بداية البرومبت

أنت مهندس سطح مكتب خبير في Rust وTauri 2 وReact وTypeScript وCodeMirror 6. طوّر Selora IDE بالاستفادة من الأفكار والخوارزميات المناسبة في OpenCursor. الهدف تجربة تحرير ذكية مترابطة، سريعة، هادئة بصريًا، وقابلة للاعتماد، مع الحفاظ على هوية Selora وبنيته الحالية.

أنت المنسّق والمسؤول عن النتيجة النهائية. استخدم subagents حقيقيين لتسريع المهام المستقلة، وفق الخطة التالية. المقصود وكلاء التنفيذ الذين يتيحهم العميل الذي تعمل داخله، وليس تشغيل ميزة subagents الموجودة داخل تطبيق Selora.

#### 0. طريقة تنفيذ واضحة باستخدام subagents

**0.1 قواعد أساسية**

1. اكتشف أدوات التفويض المتاحة فعليًا في العميل، ثم استخدم أسماءها الحقيقية. لا تكتب «شغلت وكيلًا» دون استدعاء أداة تفويض ناجح.
2. إذا كانت أدوات التفويض غير متاحة، اذكر القيد مرة واحدة، ثم نفّذ نفس بطاقات المهام بالتتابع. لا تخترع نتائج لوكلاء وهميين.
3. ابدأ بثلاثة subagents للقراءة فقط: فحص المصدر المرجعي، فحص التكامل، وفحص UI. يمكن تشغيلهم بالتوازي.
4. بعد الفحص، استخدم بحد أقصى ثلاثة منفذين متزامنين افتراضيًا. قلّل العدد إذا تشاركوا الملفات أو ظهرت منافسة على موارد البناء.
5. فوّض نتيجة صغيرة محددة، لا عبارة عامة مثل «طوّر الواجهة» أو «أضف كل ميزات Cursor». لا تمنح subagent المرحلة كلها.
6. اجعل لكل ملف كاتبًا واحدًا في أي وقت، بما في ذلك ملفات الاختبار وCSS. اسم المجلد وحده لا يكفي لتحديد الملكية.
7. اسمح بالعمل المتوازي فقط بعد تثبيت العقد الذي يربط المهام. UI وbackend لا يخترعان شكلين مختلفين للحدث نفسه.
8. لا يفوّض subagent إلى وكلاء آخرين. المنسّق وحده يدير قائمة المهام وملكية الملفات وترتيب الدمج.
9. لا تطلب من المنفذين إعادة تصميم بقية التطبيق أو إصلاح مشكلات جانبية. سجّل ما يخرج عن المهمة للمنسّق.
10. المنسّق مسؤول عن قراءة الفروق ودمجها وتشغيل التحقق المشترك. تقرير subagent «تم» لا يكفي لإعلان الميزة مكتملة.

**0.2 أدوار الفحص الأولي — قراءة فقط**

| الوكيل | المهمة المحددة | التسليم المطلوب |
|---|---|---|
| Source Analyst | اقرأ ملفات OpenCursor الخاصة بالمرحلة الأولى من جدول المراجع، وحدّد imports والاعتمادات والافتراضات | جدول: الميزة، upstream path، الدالة، ما يمكن نقله، ما يحتاج إعادة كتابة، والاختبارات المرجعية |
| Integration Analyst | تتبّع CodeMirror ثم bridge ثم Rust ثم provider والعودة، وحدّد مزامنة buffer والإلغاء والصلاحيات | مسارات ورموز موجودة، مالك النص والإصدار، seams قابلة للتوسعة، وأي تعارض مع التعديلات المحلية |
| UI Analyst | اقرأ tokens وtheme وeditor وagent surfaces والتخطيط، وافحص الواجهة إن أمكن تشغيلها | مكونات قابلة لإعادة الاستخدام، قيم التصميم الفعلية، مشاكل focus/scroll/overflow المثبتة، وخريطة موضع الميزات |

أثناء الفحص، يقرأ المنسّق تعليمات المشروع وحالة Git وأوامر التحقق. لا يرسل مهام تنفيذ تعتمد على تقارير لم تصله بعد.

**0.3 ملكية الملفات المشتركة**

هذه ملفات تكامل عالية التعارض؛ المنسّق هو كاتبها الافتراضي أثناء مرحلة التنفيذ المتوازي:

- `crates/selora-protocol/src/lib.rs`.
- `apps/selora-desktop/frontend/src/bridge/bridge.ts`.
- `apps/selora-desktop/src-tauri/src/commands.rs`.
- `apps/selora-desktop/src-tauri/src/main.rs`.
- `apps/selora-desktop/src-tauri/src/state.rs`.
- `apps/selora-desktop/src-tauri/src/sessions.rs`.
- `apps/selora-desktop/frontend/src/features/editor/CodeMirrorPane.tsx`.
- `apps/selora-desktop/frontend/src/features/agent/components/AgentComposer.tsx`.
- `apps/selora-desktop/frontend/src/features/keymap/commands.ts`.
- `apps/selora-desktop/frontend/src/styles/tokens.css`.
- ملفات تسجيل exports العامة، وإعدادات المشروع، وmanifests، وlockfiles.

يمكن نقل ملكية ملف كامل إلى منفّذ لمهمة محددة، بشرط تسجيل ذلك وإيقاف أي كاتب آخر له. يرسل subagent احتياجات تعديل الملفات المشتركة كاقتراح موضعي للمنسّق، لا يطبقها خارج نطاقه.

لا تنشئ worktrees تلقائيًا من HEAD لتجاوز التعارض؛ قد تفتقد التعديلات المحلية التي بُنيت عليها المهمة. إذا اخترت العزل، جهّز baseline يحتوي العمل الحالي دون فقده، وحدّد طريقة دمج التغييرات قبل التفويض. في workspace مشترك، استخدم ملكية الملفات بدل عزل وهمي.

**0.4 موجات التنفيذ وتبعياتها**

| الموجة | ما يجب أن يسبقها | ما يمكن تشغيله بالتوازي | عمل المنسّق بعدها |
|---|---|---|---|
| 0: الفحص | قراءة التعليمات وحالة Git | الوكلاء الثلاثة للقراءة فقط | تثبيت scope المرحلة الأولى وخريطة الملفات |
| 1: العقود | انتهاء تقارير الفحص | مراجعة عقد البيانات وUI كتابةً فقط | إضافة العقود وتوثيق الإصدارات والإلغاء؛ فحص الأنواع |
| 2: AI Tab | عقد request/result/cancel ثابت | منفّذ خدمة Rust، ومنفّذ CodeMirror extension، وكاتب اختبارات في ملفات مستقلة | ربط المسار الحقيقي، ثم اختبارات stale results وLSP وTab |
| 2.5: سجل التغييرات | عقد before/proposed/base-version ثابت | منفّذ سجل AI changes واختباراته؛ مراجعة تصميم Ctrl+K للقراءة فقط | إثبات preview/apply/reject دون كتابة preview إلى القرص |
| 3: Ctrl+K | سجل تغييرات أدنى يعمل وAI Tab اجتاز التحقق | منفّذ generation/preview service، ومنفّذ composer والتنسيق، وكاتب اختبارات مستقلة | ربط preview وAccept/Reject وUndo وAuto Save |
| 4: المراجعة | تثبيت ملفات الميزة ومنع تعديلها أثناء المراجعة | Reviewer للكود، وUI/QA للمشاهد والتفاعل | إصلاح النتائج ثم إعادة الاختبارات المتأثرة |

كاتب الاختبارات يختبر العقد والسلوك المتوقعين، وليس نسخ تنفيذ المنفّذ. يمكنه استخدام fixtures للاختبارات فقط. لا تُعرض fixtures أو providers وهميون في المنتج كمسار عامل.

قسّم المرحلة الأولى إلى بطاقات بهذا الترتيب، ثم املأ الملفات الدقيقة بعد الفحص:

- `P1-CONTRACT`: المنسّق يثبت أنواع الطلبات والنتائج والإلغاء وعقد buffer version، ثم يتحقق من توافق Rust وTypeScript.
- `P1-TAB-RUST`: خدمة توليد Tab وحدود السياق والإلغاء؛ لا تعديل UI.
- `P1-TAB-CM`: ghost decorations وأولوية لوحة المفاتيح وstale guards؛ يستهلك العقد المثبت.
- `P1-TAB-VERIFY`: حالات out-of-order وLSP وIME وCRLF وsuffix overlap في ملفات اختبار مستقلة.
- `P1-CHANGES-CORE`: سجل before/proposed وتحقق الإصدار وقرارات apply/reject؛ لا يشمل checkpoint storage أو لوحة review الكاملة بعد.
- `P1-INLINE-RUST`: generation stream وpreview lifecycle فوق `P1-CHANGES-CORE`؛ لا يفترض أن انتهاء stream يساوي قبول التعديل.
- `P1-INLINE-UI`: composer وحالاته وpreview ومفاتيح التحكم حسب القسم ط، مع اقتراح نقاط الربط للمنسّق.
- `P1-INLINE-VERIFY`: اختبارات التعديل اليدوي وAuto Save والإلغاء وUndo وdouble apply.
- `P1-INTEGRATE-QA`: المنسّق يربط ويشغّل الفحوص، ثم Reviewer وUI/QA يفحصان الناتج المستقر.

هذه IDs للمهام وليست أدوات أو أسماء agents يجب أن تكون موجودة. اكتمال كل بطاقة تنفيذ يعني نجاح فحصها المحلي وانتقالها إلى `review`؛ المنسّق يضع `done` بعد نجاح الدمج.

كرّر النمط نفسه في المراحل التالية: review/checkpoint بعد خدمة التغييرات؛ Context وRules يتوازيان بعد عقد السياق؛ MCP والفهرسة بعد عقد الأدوات؛ خدمات Terminal وCLI وGit تتوازى فقط إذا لم تتشارك الملفات. حافظ على مالك واحد لـSettings وComposer وrouter وbridge.

**0.5 بطاقة مهمة إلزامية لكل subagent**

أرسل بطاقة مكتملة ومفهومة دون الاعتماد على سياق المحادثة الرئيسي:

```text
Task ID: <مثال: P1-TAB-CM>
Goal: <سلوك واحد يمكن للمستخدم تجربته>
Mode: <read-only أو implementation أو review>
Workspace: <المسار الحقيقي>
Baseline: <نسخة العمل الحالية وأي تعديلات محلية مهمة>
Read first: <المسارات والدوال المطلوبة، بما فيها مرجع OpenCursor المحدد>
Allowed writes: <قائمة ملفات دقيقة؛ اذكر الملفات الجديدة المقترحة صراحةً>
Do not write: <ملفات التكامل التي يملكها المنسّق>
Depends on: <Task IDs المنتهية أو عبارة none>
Contract: <نسخة العقد، الحقول، أنواع الأحداث، ترتيبها، ودلالة الإلغاء والأخطاء>
UI requirements: <الحالات، المقاسات، الاختصارات، focus وscroll إن وجدت>
Acceptance: <خطوات تجربة واضحة ونتائج قابلة للتحقق>
Checks: <أوامر الاختبار ذات الصلة>
Return: <الملفات المعدلة، قرارات التنفيذ، مصدر الاقتباس، نتائج الفحص، واحتياجات الدمج>
Stop condition: <إذا احتجت تغيير العقد أو ملفًا خارج ملكيتك، أبلغ المنسّق قبل التعديل>
```

لا تترك placeholders في بطاقة مرسلة. لا تطلب من subagent استنتاج أسماء DTOs أو كتابة protocol خاص به. عند تغير العقد، حدّث رقمه وأبلغ جميع المستهلكين قبل استئنافهم.

**0.6 الدمج وسجل التقدم**

- أنشئ سجلًا صغيرًا مثل `SELORA_UPGRADE_PROGRESS.md` إن لم يوجد سجل مناسب. المسار مقترح جديد، وليس ملفًا موجودًا بالضرورة.
- لكل مهمة سجّل: ID، المسؤول، الملفات المملوكة، dependencies، contract version، الحالة، ونتيجة التحقق.
- الحالات: `todo`, `running`, `blocked`, `review`, `done`. لا تستخدم `done` قبل الربط والاختبار.
- بعد تسليم المهمة، اقرأ diff والاعتمادات وcleanup paths، ثم ادمجها مع التطبيق. لا تستبدل ملفًا كاملًا بإصدار subagent دون مقارنة.
- شغّل اختبارات integration على نسخة مستقرة من الملفات، لا بينما منفّذ آخر يغيّر الملفات المعنية.
- لا تشغّل عدة عمليات build شاملة أو Cargo على target directory نفسه بالتزامن. الاختبارات الصغيرة المستقلة يمكن موازاتها.
- لا تعالج فشل التحقق باستخدام `any` أو `@ts-ignore` أو تعطيل test أو lint rule؛ أصلح العقد أو السلوك المسبب.
- لا تضف dependency إلا لحاجة محددة. المنسّق يحدّث manifest وlockfile معًا قبل إعادة التحقق باستخدام `--locked`.
- عند بلوغ حد السياق، سجّل ما يعمل، المهام الجارية، الملفات المملوكة، الأوامر الناجحة، والخطوة التالية. لا تبدأ موجة جديدة بلا مساحة لإغلاقها.

#### أ. المراجع والنطاق

المشروع المستهدف هو مجلد Selora IDE الحالي. المرجع هو:

- Repository: `https://github.com/noah-thing/open-cursor`.
- Commit: `2ac2b8fd7dde25d6c574a109913c874f9c3b959c`.
- Local reference: `C:\Users\Toxic\AppData\Local\Temp\opencode\open-cursor-reference-20261101`.
- اقرأ خريطة الاقتباس والمقارنة في `SELORA_OPEN_CURSOR_PROMPT.md`.

إذا لم يعد المرجع المحلي موجودًا، اجلب النسخة المحددة إلى مجلد مرجعي منفصل. لا تشغّل install scripts للمشروع المرجعي؛ المطلوب قراءة الكود وتكييفه.

التطبيق المستهدف هو `apps/selora-desktop`، وليس نقل الواجهة إلى Electron أو Code OSS. يمكن إعادة استخدام منطق Rust الموجود في crates، لكن اكتمال ميزة في egui لا يعني أنها تعمل في CodeMirror.

ابدأ بقراءة تعليمات المشروع و`git status` والتعديلات المحلية. توجد أعمال قيد التطوير تشمل Agents workspace والمسودات والمزامنة؛ افهمها قبل لمس الملفات. لا تستبدل تعديلات المستخدم ولا ترجعها إلى HEAD. لا تنشئ commit تلقائيًا.

#### ب. افحص الأساس ثم حدّد عقود التكامل

راجع على الأقل:

- `apps/selora-desktop/frontend/src/features/editor/CodeMirrorPane.tsx`.
- `apps/selora-desktop/frontend/src/features/editor/editor-store.ts`.
- `apps/selora-desktop/frontend/src/features/editor/pending-edits.ts`.
- `apps/selora-desktop/frontend/src/features/editor/text-offsets.ts`.
- `apps/selora-desktop/frontend/src/features/agent/components/AgentComposer.tsx`.
- `apps/selora-desktop/frontend/src/features/agent/components/ToolCard.tsx`.
- `apps/selora-desktop/frontend/src/features/agent/agent-store.ts`.
- `apps/selora-desktop/frontend/src/features/agents-workspace`.
- `apps/selora-desktop/frontend/src/bridge/bridge.ts`.
- `apps/selora-desktop/frontend/src/features/keymap/commands.ts`.
- `apps/selora-desktop/frontend/src/features/dock/TerminalPanel.tsx`.
- `apps/selora-desktop/frontend/src/styles/tokens.css`.
- `apps/selora-desktop/frontend/src/styles/palette.css`.
- `apps/selora-desktop/frontend/src/styles/base.css`.
- `apps/selora-desktop/frontend/src/styles/animations.css`.
- `apps/selora-desktop/frontend/src/features/theme/theme.ts`.
- `apps/selora-desktop/frontend/src/components/icons/index.ts`.
- `apps/selora-desktop/frontend/src/features/agents-workspace/AgentsChangesView.tsx`.
- `apps/selora-desktop/frontend/src/features/sidebar/GitPanelView.tsx`.
- `apps/selora-desktop/src-tauri/src/sessions.rs`.
- `apps/selora-desktop/src-tauri/src/state.rs`.
- `crates/selora-protocol/src/lib.rs`.
- `crates/selora-ai/src/client.rs`.
- `crates/selora-ai/src/agent/router.rs`.
- `crates/selora-settings/src/settings.rs`.

اكتب خريطة قصيرة: الموجود، الجزئي، والجديد. لكل ميزة اذكر مالك الحالة ومسار الحدث من الواجهة إلى Rust والعكس. لا تجعل `state.rs` أو `sessions.rs` مستودعًا لكل المنطق الجديد؛ استخرج خدمات محددة عند الحاجة.

اجعل Rust مصدر الحقيقة للتخزين وعمليات الملفات والطلبات الخارجية والعمليات الفرعية. اجعل الواجهة مسؤولة عن العرض والتفاعل وحالة العرض المؤقتة. استخدم بروتوكول Selora وأحداثه بدل إضافة قناة موازية غير متوافقة.

اربط كل طلب بالهويات المناسبة: `workspace_id`, `session_id`, `request_id`, `buffer_id`, ونسخة المستند. تجاهل الأحداث القديمة بعد الإلغاء أو تبديل المشروع أو إغلاق المستند. حافظ على توافق البيانات والجلسات القديمة مع الحقول الجديدة.

**قائمة تثبيت العقد قبل إرسال مهام التنفيذ:**

1. حدّد اسم الأمر والحدث الفعليين ومكان تعريف النوع في Rust وTypeScript. أعد استخدام envelope الحالي متى كان مناسبًا.
2. حدّد أي نسخة نص يستخدمها الطلب: نسخة CodeMirror المحلية أم buffer backend الذي أُقرّت مزامنته. لا تخلط العدّادين.
3. إذا كان النص المحلي أحدث من backend، استخدم snapshot صريحة أو انتظر الإقرار حسب العقد. لا تبدأ من نص قديم بصمت.
4. حدّد الحقول اللازمة لكل ميزة فقط؛ AI Tab لا يحتاج session محادثة مصطنعة إذا كان الطلب مستقلًا.
5. حدّد ترتيب stream chunks وكيفية اكتشاف التكرار، وحدث terminal واحد لكل request. امنع terminal متأخرًا من إعادة فتح طلب ملغى.
6. حدّد response للنجاح والإلغاء والخطأ والتعارض، مع message مفهومة وretryability. لا تُمثّل جميعها بـboolean.
7. حدّد من يملك Abort/CancellationToken ومتى تُحرر subscriptions وtimers وprocess handles.
8. حدّد idempotency لقبول التعديل ورفضه. النقر المزدوج أو event مكرر لا يطبق النص مرتين.
9. حدّد نافذة التطبيق المخوّلة بالتطبيق. Main وAgents قد يعرضان الحالة، لكن لا ينفذان الأثر نفسه مرتين.
10. أثبت التوافق بfixture JSON واقعية واختبار serialization/deserialization عندما تتغير العقود، ثم جمّد الإصدار للمنفّذين.

#### ج. الأساس المشترك لتعديلات AI

صمّم سجل تغييرات صغيرًا يمكن أن يستخدمه Ctrl+K والوكيل والمراجعة وcheckpoints. عرّف قبل التنفيذ:

- before-content، النص المقترح، مصدر التغيير، الدور والجلسة والملف وإصدار الأساس.
- حالات generation وpreview وaccepted وrejected وconflict وcancelled وfailed.
- متى يصبح التغيير تعديلًا فعليًا في buffer، ومتى يُكتب إلى القرص.
- علاقة Accept / Reject بـUndo / Redo وAuto Save وdirty buffers.
- طريقة المحافظة على edits المستخدم التي حدثت بعد بدء الطلب.

لا تستخدم `features/editor/pending-edits.ts` لتخزين review state؛ وظيفته الحالية مزامنة المحرر. أنشئ تسمية واضحة مثل `ai-changes`، واجعل أي تفاعل مع المزامنة يمر عبر عقد صريح.

افصل الإذن بالعملية عن مراجعة نتيجتها. Ask وAgent وPlan لها سلوك قائم في Selora، ولا يجوز تغيير معنى أي منها أثناء إضافة مراجعة التعديلات.

#### د. المرحلة الأولى: AI Tab وCtrl+K

**AI Tab**

1. فعّل ghost text داخل CodeMirror باستخدام extension مناسب؛ لا تعِد إنشاء `EditorView` مع كل تحديث.
2. استفد من `ghost_complete` الموجود قبل تقرير توسيعه. نسخته الحالية تعتمد prefix وchat؛ أضف suffix وFIM حسب قدرات المزوّد.
3. استخرج ما يلزم من `tab/logic.ts`: نافذة السياق، معالجة indentation، إزالة fences، منع suffix overlap، وcache محدود.
4. وفّر إعداد نموذج مستقل لـTab مع fallback واضح إلى نموذج مناسب. لا تفترض FIM أو vision من اسم النموذج.
5. استخدم debounce قابلًا للضبط، وقيمة أولية مستهدفة 100–200ms. ألغِ الطلب السابق عند تغيّر المستند أو المؤشر.
6. لا تعرض نتيجة إلا إذا تطابقت هوية المشروع والمستند وإصدار النص والموضع والسياق مع الطلب.
7. خارج IME، ترتيب Tab: قائمة إكمال LSP المفتوحة، ثم التنقل داخل snippet نشط، ثم ghost text صالح مع تحديد فارغ، ثم indentation المعتاد. لا تسمح لمسارين باستهلاك الضغطة نفسها. Escape يخفي ghost text عندما يكون هو التفاعل النشط.
8. لا تطلب اقتراحًا أثناء IME composition، ولا تجعل قبول ghost text يحذف تحديدًا أو يغير نصًا غير مقصود.
9. لا تستدعِ النموذج لكل حرف، ولا تنفّذ serialization كاملًا لملف كبير لكل keystroke.

**Ctrl+K داخل المحرر**

1. افتح composer مدمجًا قرب التحديد، مع prompt ونموذج وGenerate / Stop.
2. مع تحديد: اقترح استبداله. دون تحديد: اقترح إدراجًا عند المؤشر. أظهر نطاق العملية بوضوح.
3. استخدم `inline/region.ts` و`inline/fence.ts` كأساس للفهم؛ لا تنسخ توسيع النطاق إلى أسطر كاملة دون اختيار UX مقصود.
4. اعرض streaming في طبقة preview أو diff لا تكتب أجزاء غير مكتملة إلى القرص عبر Auto Save.
5. وفّر Accept / Reject / Retry وتعليمات متابعة على الاقتراح الحالي.
6. طبّق أصغر transaction ممكنة بدل استبدال المستند كاملًا. حافظ على selection وscroll وfolds خارج النطاق.
7. اجعل قبول الاقتراح خطوة Undo مفهومة. رفض الاقتراح لا يتراجع عن كتابة المستخدم اليدوية.
8. عند تغيير المستخدم للنطاق نفسه أثناء التوليد، أوقف التطبيق التلقائي وأظهر conflict قابلًا للحل.
9. اختبر العربية وemoji وCRLF وUTF-16؛ لا تفترض تطابق byte offsets في Rust مع مواضع JavaScript.

معيار الإنجاز: يعمل المساران من CodeMirror إلى المزوّد والعودة، مع إلغاء فعلي واختبارات stale responses. لا تُعتبر واجهة demo أو ghost text ثابتة إنجازًا.

#### هـ. المرحلة الثانية: مراجعة التعديلات وcheckpoints

**مراجعة التعديلات**

- اعرض Changes للدور أو الجلسة، مع عدد الملفات والأسطر المضافة والمحذوفة.
- افتح الملف عند hunk المحدد، ووفّر Keep / Undo لكل hunk، لكل ملف، وللمجموعة المختارة.
- استفد من `edits/hunks.ts` وراجع اعتماد `diff`. لا تطبّق hunk اعتمادًا على رقم السطر وحده بعد تغير المستند.
- اجعل تحديث baseline بعد Keep منفصلًا عن undo للنص الحالي بعد Reject.
- احتفظ بهوية الأجزاء قدر الإمكان بعد اتخاذ قرار على أجزاء أخرى، وأعد التحقق قبل كل تطبيق.
- اجمع parent وsubagent edits مع إظهار مصدر كل تغيير. لا تنسب تغييرات جلسة أخرى إلى الدور الحالي.
- حافظ على diff وGit staging الحاليين؛ مراجعة AI لا تنفّذ stage أو commit تلقائيًا.

**Checkpoints الملفات**

- أنشئ checkpoint مرتبطًا برسالة المستخدم قبل أول side effect للملفات.
- التقط أول before-content لكل ملف في الدور، مع تمييز الملف غير الموجود عن الملف الفارغ.
- استخدم content-addressed storage خارج شجرة المصدر القابلة للفهرسة، أو مسار `.selora` مستبعدًا بوضوح.
- افصل نسخة buffer غير المحفوظة عن نسخة القرص؛ لا تفقد dirty edits عند الالتقاط أو الاستعادة.
- وفر preview للاستعادة يوضح الملفات المتأثرة والتعارضات.
- إذا تغيّر الملف يدويًا أو من جلسة أخرى، لا تستبدله بصمت. استخدم مقارنة ثلاثية أو conflict واضحًا.
- سجّل تقدم الاستعادة، واحتفظ ببياناتها عند الفشل الجزئي، واجعل إعادة المحاولة idempotent.
- لا تستخدم `git reset --hard` لتنفيذ الاستعادة.
- لا تدّعِ التراجع عن أوامر shell أو آثار الشبكة؛ snapshot الملفات لا يعكس side effects خارج الملفات التي جرى تتبعها.
- اترك حماية `retry_partial_effects` حتى تتوفر دلالة صحيحة ومختبرة للاستعادة وإعادة التنفيذ.
- لا تسمِّ Restore للمحادثة فقط «Restore project».

معيار الإنجاز: دور يغيّر ملفين وينشئ ثالثًا، ثم استعادة صحيحة بعد إعادة تشغيل التطبيق، مع اختبار conflict وفشل جزئي.

#### و. المرحلة الثالثة: Context وRules

**Context picker**

أضف قائمة @ قابلة للاستخدام بلوحة المفاتيح، مع نتائج مجمعة وchips قابلة للمعاينة والإزالة. ابدأ بالملفات والتحديد وGit وProblems وTerminal، ثم الرموز والمجلدات والتوثيق والمحادثات السابقة والصور.

- عرّف DTO typed لكل نوع بدل تحويل كل شيء إلى prompt نصي مبهم.
- خزّن provenance: المصدر، path/range، الإصدار أو وقت الالتقاط، وحالة truncation.
- لا ترسل بيانات المشروع كاملة لمجرد إرفاق مجلد؛ أرسل ملخصًا محدودًا ثم اقرأ المحتوى المطلوب.
- التحديد يجب أن يمثل النص الذي اختاره المستخدم، بما فيه النص غير المحفوظ. وسّع العقد الحالي صراحةً ولا تفترض أن `read_file` يرى buffer.
- إذا لم توجد document symbols في جسر LSP الحالي، أضف العقد أو عطّل النوع بوضوح إلى حين دعمه.
- لمخرجات Terminal، ميّز «آخر مخرجات محددة» عن «آخر أمر»؛ لا تخترع حدود أوامر دون shell integration.
- اعرض تقدير حجم السياق بصفة estimate، ولا تعرض token count دقيقًا بلا tokenizer موثوق.
- للصورة: paste/drop وpreview وحدود حجم، ثم content block حقيقي إلى نموذج يدعم الصور. اشرح السبب إذا كان النموذج لا يدعمها.
- اجعل المرفقات والمسودات مستقلة حسب المشروع والجلسة ومتوافقة مع نافذتي Main وAgents.

**Rules وSlash Commands**

- اقرأ `AGENTS.md` و`CLAUDE.md` و`.selora/rules/*.{md,mdc}`، مع دعم نطاقات glob والملفات المتداخلة.
- حدّد ترتيب المصادر والأولوية والتكرار في مكان واحد، واعرض أصل كل rule فعالة.
- وفّر توافقًا اختياريًا مع `.cursor/rules` و`.open-cursor/rules` دون تحويل أسماء ملفات المستخدم بصمت.
- استخدم cache مع invalidation عند تغيير القواعد أو المشروع.
- أضف `.selora/commands/*.md` وقائمة `/` مع description وpreview وتوسيع template قبل الإرسال.
- هذه templates للمحادثة، وليست shell scripts تُنفَّذ عند اكتشافها.
- أضف صفحة Rules تبين matched rules وسبب تطبيقها بدل إعدادات شكلية غير مربوطة.

#### ز. المرحلة الرابعة: MCP والبحث الدلالي

**MCP**

- أنشئ خدمة في Rust تدير دورة حياة clients، بدءًا بـstdio وStreamable HTTP. أضف SSE فقط إذا احتجته للتوافق.
- استخدم `.selora/mcp.json` وإعدادات المستخدم، مع ترتيب merge موثق، ودعم استيراد الإعدادات المتوافقة.
- اعرض disconnected / connecting / ready / error / disabled وقائمة الأدوات، مع Connect / Restart / Disable.
- أدخل الأدوات في manifest وrouter الموجودين، بأسماء تمنع التعارض مع الأدوات الأصلية وبين servers.
- طبّق timeout والإلغاء وتنظيف العمليات عند إغلاق المشروع، ولا تعلّق واجهة المستخدم أثناء الاتصال.
- حافظ على سياسة الموافقات الحالية عند تشغيل عملية أو استدعاء أداة؛ إعداد مشروع جديد لا يغيّر صلاحيات الجلسة بصمت.
- مرّر credentials عبر آلية الأسرار الموجودة، لا عبر frontend store أو سجل الأحداث.

**الفهرسة والبحث**

- نفّذ index تدريجيًا خارج UI thread، مع حالة progress وPause / Resume / Rebuild.
- احترم `.gitignore` وسياسة استبعاد المشروع الموجودة، وتحقق من دور `.zcodeignore` قبل إضافة صيغة أخرى.
- استبعد binary وbuild output و`node_modules` و`target` ومخازن الجلسات والcheckpoints والملفات الحساسة.
- اربط كل chunk بمسار وأسطر وإصدار محتوى، وحدّث create/change/delete/rename.
- اجعل اختيار embedding provider واضحًا: محلي أو بعيد. التخزين المحلي للvectors لا يعني أن embedding computation محلي.
- أعد فهرسة ما يلزم عند تغيير embedding model أو vector dimensions.
- ابدأ بحدود واضحة للملفات والتوازي والذاكرة. لا تنقل vector search خطيًا إلى WebView لمشروع كبير.
- وفّر `codebase_search` للوكيل بنتائج قابلة للاستشهاد ومسارات تفتح عند الأسطر الصحيحة.
- استخدم البحث النصي الحالي كfallback واعرض ذلك بوضوح؛ لا تسمِّ نتائج grep نتائج دلالية.

#### ح. المرحلة الخامسة: Terminal وCLI agents والنماذج المحلية وGit

**Terminal AI**

- Ctrl+K داخل Terminal يفتح إدخالًا صغيرًا، ويرسل shell الفعلي وcwd والسياق المحدود.
- يعرض أمرًا قابلًا للتعديل. Insert يلصقه دون تنفيذ؛ التنفيذ يظل فعلًا صريحًا من المستخدم.
- احترم الفرق بين Windows PowerShell وPowerShell 7، خصوصًا دعم `&&` والاقتباس والمسارات ذات المسافات.
- لا تنقل `cleanCommand` حرفيًا إذا كان دمج الأسطر سيغيّر معنى PowerShell.
- استفد من xterm buffer أو parser مناسب للمخرجات؛ اختبر ANSI sequences المقسّمة بين chunks.

**CLI agents**

- ابدأ بـClaude Code وCodex المثبتين، مع detection للإصدار وحالة الاستخدام الفعلية.
- عرّف adapter موحدًا: start، stream events، cancel، resume، session mapping، file changes، tool events.
- استخدم المصادقة التي يديرها CLI؛ لا تستخرج tokens أو تنشئ login flow مقلدًا.
- طابق قدرات كل CLI مع الواجهة؛ لا تعرض خيار permission أو model لا يدعمه الإصدار المثبت.
- cancellation ينهي process tree الذي بدأته الجلسة على Windows، ولا يترك child process يكتب بعد Stop.
- أدخل التغييرات في review/checkpoints حيث يمكن ضمان before-content، وأظهر أي حدود tracking بصدق.

**Local models**

- وسّع إعداد Ollama الحالي بدل إنشاء مزوّد مكرر. أضف اكتشاف الخدمة والنماذج وربط LM Studio عند توفره.
- افحص RAM وVRAM والمساحة، ثم قدّم توصيات تقريبية مع سببها. الملاءمة الحسابية لا تضمن سرعة أو دعم tools.
- لا تنسخ أسماء النماذج وأحجامها من catalog المرجع على أنها حقائق دائمة؛ تحقق من المصدر وقت الاستخدام.
- أظهر download progress والإلغاء والأخطاء، واسمح باختيار نموذج موجود دون إعادة تنزيله.
- افصل نموذج Chat عن Tab وInline وEmbeddings، مع defaults مفهومة دون إجبار المستخدم على أربعة إعدادات.

**Commit message**

- أضف Generate بجوار حقل الرسالة في Git panel.
- استخدم staged diff افتراضيًا، وحدّد نطاقًا واضحًا إذا لم توجد staged changes.
- راعِ أسلوب commits الحديثة وقواعد المشروع، مع ميزانية للمحتوى.
- لا تستبدل نص المستخدم إذا كتبه أثناء التوليد؛ اعرض الاقتراح كخيار.
- لا تنفّذ commit أو stage أو push تلقائيًا.

#### ط. تعريف «سلسة» القابل للقياس

السلاسة ليست زيادة المؤثرات. المطلوب أن يشعر المستخدم بأن المحرر يرد مباشرةً ولا يفقد مكانه أو عمله.

**ط.1 الاتجاه البصري الملزم**

صمّم الواجهات الجديدة كأدوات IDE احترافية: محتوى واضح، كثافة مناسبة، عناصر قليلة، وتباين مريح. راحة العين تعني نصًا مقروءًا وترتيبًا هادئًا، وليس تخفيض opacity للنص حتى يصعب قراءته.

| العنصر | المواصفة المطلوبة |
|---|---|
| الأسطح | استخدم semantic tokens الموجودة: `--ink-canvas`, `--ink-low`, `--ink-mid` أو النظائر الفعلية في المكوّن الحالي. افحص قيمها المحسوبة في الثيم؛ لا تضف palette منفصلة |
| الألوان | لون accent واحد للتفاعل الأساسي، وألوان الحالات من tokens. diff يستخدم tint خفيفًا مع `+` و`-` وتسميات؛ لا تعتمد على اللون وحده |
| النص الأساسي | `--font-sans` مع `--type-md` و`--type-md-lh`، أي 13/20px قبل font scaling |
| النص الثانوي | `--type-sm` و`--type-sm-lh`، أي 12/18px قبل scaling. احصر `--type-xs` في metadata الصغيرة، لا نص الخطأ أو وصف الإجراء |
| العناوين | `--type-lg` للعناوين المحلية، وأوزان 400/500/600 حسب المستوى. لا تستخدم عناوين ضخمة داخل popover |
| الكود والمسارات | `--font-mono` عند الحاجة. احفظ اتجاه LTR للكود والمسارات، واستخدم bidi isolation داخل المحتوى العربي المختلط |
| المسافات | استخدم `--space-1` إلى `--space-5`: 4/8/12/16/24px. gap 8px داخل الصف، padding 12–16px داخل composer، ومسافة 16px بين مجموعات الإعدادات |
| الزوايا والحدود | استخدم radius وborder tokens الحالية؛ لا تفرض تدويرًا كبيرًا على كل صف، ولا تضف بطاقة داخل بطاقة دون وظيفة |
| الأزرار | زر أساسي واحد لكل حالة. min-height مستهدف 32px للأزرار العادية، ومنطقة نقر 28×28px لأيقونات الأدوات، مع نمو المحتوى عند تكبير الخط |
| الأيقونات | أعد استخدام `components/icons` بنفس نمط stroke؛ استهدف 16px في الأدوات. كل icon-only button له accessible name وtooltip قابل للظهور بالتركيز |
| التباين | تحقّق من 4.5:1 للنص العادي المطلوب قراءته، و3:1 لحدود التركيز والعناصر الرسومية الضرورية. اختر token أو alias مناسبًا بدل تكرار ألوان literal |
| التفاصيل المتقدمة | أخفِها في disclosure أو قائمة واضحة؛ لا تعرض model وeffort وlogs وtoken statistics كاملة في كل صف |
| الأخطاء | رسالة محلية قصيرة تشرح ما حدث والخطوة التالية، مع تفاصيل تقنية قابلة للفتح. لا تجعل stack trace هو النص الافتراضي |

الأبعاد الجديدة أهداف للمكونات الجديدة، وليست أسماء tokens موجودة. عرّف أي token جديد عبر المنسّق بعد مراجعة الموجود. استخدم `min-height` بدل ارتفاع ثابت يمكن أن يقص النص، واحترم font scale الحالي 1 و1.15 و1.3.

لا تضف إلى الأسطح الجديدة neon أو وهجًا نابضًا أو animated gradients أو blur واسعًا أو bounce. وجود effects قديمة في Selora لا يلزم تكرارها في كل ميزة. حافظ على العلامة والهوية الموجودة، واجعل الكود والقرار الذي يتخذه المستخدم مركز الاهتمام.

**ط.2 مواقع المكونات وحدودها**

- **AI Tab:** ghost text داخل المحرر فقط. استخدم لونًا ثانويًا مقروءًا، ولا تحرّك caret أو المؤشر. لا toast أو spinner مع كل اقتراح. إعداد التفعيل والنموذج في Settings.
- **Inline composer:** قرب التحديد داخل مساحة المحرر، وليس dialog يحجب الشاشة. العرض المستهدف 440px، ويقل تلقائيًا حتى يترك 12px من حواف المساحة المتاحة. لا تفرض `min-width` يسبب overflow في split ضيق.
- **Inline composer layout:** صف عنوان صغير يوضح الملف والنطاق مع Close؛ prompt يبدأ بسطرين ويتمدد حتى ستة أسطر ثم يصبح قابلاً للتمرير؛ footer للنموذج وحالة الطلب والإجراء الأساسي.
- **Preview:** تمييز الأسطر المقترحة داخل المحرر مع summary مختصر. بعد اكتمال التوليد تظهر Accept وReject وRetry. لا تجعل المستخدم يبحث عن زر القبول في panel بعيدة.
- **Review:** استخدم سطح Changes الموجود أو تبويبًا واضحًا داخله باسم AI Changes. اجعل Git Changes وAI Changes مميزين بوضوح مع baseline/session label؛ لا تخلط قائمتيهما.
- **قائمة @:** anchored إلى composer أو caret، تعرض حتى ثمانية صفوف تقريبًا قبل scrolling. تبقى داخل النافذة وتنتقل للأعلى إذا ضاقت المساحة أسفلها.
- **Attachment chips:** صفان مرئيان افتراضيًا ثم `+N` لكشف الباقي. يحتفظ chip باسم مفهوم وRemove، مع tooltip للمسار الكامل، دون تمديد عرض النافذة.
- **MCP وRules وLocal Models:** أقسام ضمن Settings والتنقل القائم. أظهر التفاصيل عند اختيار العنصر؛ لا تضف شريطًا جانبيًا دائمًا لكل خدمة.
- **Terminal AI:** composer مرتبط بالـTerminal النشط. المعاينة لا ترسل newline إلى PTY، ولا تفتح Terminal جديدة بلا حاجة.

أعد استخدام primitives الموجودة إن وُجدت. إن لم توجد، استخرج أصغر مكوّن مشترك تحتاجه الأسطح فعلًا؛ لا تدخل UI framework كاملًا من أجل popover واحد.

**ط.3 حالات Ctrl+K الدقيقة وسلوك الأزرار**

الأسماء التالية حالات سلوك مقترحة، وليست DTOs موجودة. ثبّتها مع عقد التطبيق قبل البرمجة:

| الحالة | ما يظهر | الإجراءات الممكنة | شرط الانتقال |
|---|---|---|---|
| idle | prompt قابل للتحرير والنطاق المختار | Generate، Close | Generate يعمل عند وجود نص ونموذج واتصال مناسبين |
| generating | حالة قصيرة مثل Generating، ومعاينة streaming | Stop؛ بقية prompt لا يطلق طلبًا ثانيًا | completion إلى preview؛ خطأ إلى error؛ تغير النطاق إلى conflict |
| cancelling | Stopping في الموضع نفسه دون spinner إضافي | لا تكرار للإلغاء ولا قبول للنص الجزئي | عند الإقرار أو إبطال generation محليًا إلى stopped |
| stopped | prompt محفوظ، ولا اقتراح جزئي قابل للتطبيق | Retry، Close | Retry ينشئ request جديدًا من snapshot جديدة |
| preview | الاقتراح الكامل وملخص diff | Accept، Reject، Retry، أو follow-up | Accept يتحقق مجددًا من إصدار الأساس قبل applying |
| applying | Applying مع تثبيت موضع الأزرار | تعطيل قبول/رفض مكرر لهذا الطلب | إقرار backend إلى accepted؛ mismatch إلى conflict؛ خطأ إلى error |
| accepted | feedback قصير هادئ ثم إغلاق composer | Undo الطبيعي في المحرر | أعد focus إلى المحرر بعد التأكد من نجاح التطبيق |
| rejected | إزالة الاقتراح دون لمس baseline أو كتابة المستخدم | يعود composer إلى idle مع حفظ prompt | لا تعرض نجاح تطبيق لم يحدث |
| error | رسالة محلية وprompt محفوظ | Retry إن كان ممكنًا، Details، Close | لا يُطبق جزء من إجابة ناقصة تلقائيًا |
| conflict | توضيح أن النص الأساسي تغير | Regenerate من النص الحالي، أو Discard preview | لا Apply أعمى فوق التعديلات اليدوية |

عند إعادة توليد follow-up من preview صالح، احتفظ بالـpreview السابق كحالة مستقلة. إذا فشل الطلب الجديد أو توقف، أعد العرض السابق بوضوح، ولا تستبدله بإجابة جزئية.

- Enter في prompt يبدأ Generate عندما تكون الحالة idle والنص صالحًا؛ Shift+Enter سطر جديد. أثناء IME لا يرسل Enter الطلب.
- قبول preview بواسطة Ctrl+Enter فقط عندما يكون focus داخل composer الخاص به وتكون الحالة preview. لا تغيّر Ctrl+Enter في Git أو chat.
- Escape يغلق أعلى قائمة فرعية أولًا. أثناء generation يطلب Stop. في preview يغلق composer مع حفظ الاقتراح في حالة المراجعة؛ الإغلاق ليس Reject.
- قبل وجود لوحة Review الكاملة في المرحلة الثانية، يسمح Ctrl+K على الملف نفسه بإعادة فتح preview المعلّق. اجعل Close متوافقًا مع Escape في الحالة نفسها، ولا تترك اقتراحًا مخفيًا بلا طريقة للوصول إليه.
- بعد Escape أثناء generation، إذا كانت العملية معلّقة فاضبط مهلة الإلغاء وأبطل generation محليًا؛ أي chunk متأخر يُهمل.
- الطلب الذي توقف لا يصبح accepted. إخفاء مؤشر التحميل لا يثبت انتهاء العملية الفرعية؛ نظّف الموارد في backend وفق العقد.
- Retry ينشئ `request_id` جديدًا ويقرأ الحالة الحالية. لا يعيد إرسال قرار كتابة سابق كما هو.

**ط.4 التخطيط وfocus وscroll**

1. لكل panel scroll owner واحد للمحتوى الأساسي. اجعل `min-width: 0` و`min-height: 0` في أبناء flex/grid الذين يحتاجون shrink وscroll.
2. لا تضع `overflow: hidden` عشوائيًا لعلاج قص popover. استخدم طبقة overlay القائمة داخل نافذة WebView نفسها مع clipping/positioning صحيحين.
3. حدّد طبقات z-index مرة واحدة؛ popover فوق المحتوى، dialog فوق popover، وtooltip لا يغطي الإجراء الذي يشرحه.
4. عند تحريك divider، يتبع الحجم المؤشر مباشرة. لا تضع transition على width/height أثناء resize، ولا تستدعِ layout reads لكل mousemove دون batching.
5. أعد حساب موضع popover عند resize وscroll ومقياس الخط، دون إعادة إنشاء CodeMirror أو تغيير مستنده.
6. inline composer غير modal ولا يحبس Tab داخل نفسه. dialog الحقيقي فقط يدير focus trap، ويعيد focus للمصدر بعد الإغلاق.
7. في @ picker: ArrowUp/ArrowDown يختاران، Enter يرفق الخيار، Escape يغلق. لا ترسل رسالة chat عند Enter إذا كانت القائمة تملك التفاعل.
8. احتفظ بالمحتوى المحدد وموضع scroll عند تحديث النص المعروض أو رجوع focus. لا تستخدم `scrollIntoView` على حاوية تجعل النافذة كلها تتحرك.
9. اعرض أسماء الملفات الطويلة باختصار بصري وtooltip، مع بقاء الاسم الكامل متاحًا لقارئ الشاشة والنسخ.
10. overflow أفقي مسموح داخل code/diff عند الحاجة، وليس على التطبيق كله. استخدم CSS logical properties للتباعد حتى يبقى النص العربي المختلط صحيحًا.

**ط.5 الحركة والرسم ودورة الحياة**

1. حافظ على design tokens والألوان والمسافات ونمط الأزرار الموجود. واجهات الأدوات الجديدة هادئة وصغيرة.
2. استخدم `--dur-quick` الحالي 160ms و`--ease-calm` أو `--ease-precise` لفتح الأدوات الجديدة. الحركة opacity أو translation صغيرة بحد أقصى 4px، دون scale أو spring. hover يمكن أن يستخدم `--dur-instant` الحالي 80ms.
3. عند فتح composer صراحةً انقل focus إلى prompt مرة واحدة. تحديثات streaming والحالة لا تنقل focus ولا تقفز بالصفحة.
4. أثناء streaming حدّث الجزء النشط فقط؛ لا تعِد parsing/rendering كامل سجل المحادثة مع كل token.
5. اجمع دفعات العرض خلال 50–100ms مبدئيًا، ثم اضبطها بناءً على profiling. لا تضف تأخيرًا ثابتًا لقبول الاقتراحات المحلية.
6. حافظ على scroll لكل جلسة. اتبع الرد فقط عندما يكون المستخدم قريبًا من الأسفل؛ وإلا أظهر Jump to latest.
7. راجع اختبار `AgentExecutionFeed.scroll.test.tsx` الموجود قبل أي تعديل على هذا السلوك.
8. استخدم virtualization للقوائم الطويلة حيث يثبت profiling الحاجة، مع الحفاظ على text selection والارتفاعات واسترجاع الموضع.
9. اجعل الأزرار والmenus قابلة للوصول بلوحة المفاتيح، وأعد focus إلى مصدر الفعل بعد الإغلاق.
10. افصل زمن UI المحلي عن زمن الشبكة والنموذج. سجّل p50 وp95 لكليهما دون تسجيل محتوى المستخدم.
11. لا تستخدم `transition: all` أو GPU hints دائمة على كل عنصر. لا تضف `will-change` إلا لعنصر يتحرك فعلًا ولمدة الحركة اللازمة.
12. عند reduced motion أوقف loops واحذف حركة التنقل المكانية في الأسطح الجديدة؛ يمكن إبقاء fade القصير وفق نظام المشروع. احترم إعداد OS وإعداد Selora معًا.
13. تظهر حالة loading النصية مباشرةً؛ يمكن تأخير spinner نحو 150ms لتجنب flash للعمليات القصيرة. لا تؤخر بدء الطلب أو الاستجابة نفسها.
14. لا تعِد mount للمحرر أو composer أو Terminal عند تبديل loading/preview/error. الهوية والمفاتيح ثابتة ما دام العنصر نفسه موجودًا.
15. ألغِ timers وsubscriptions وResizeObservers وlisteners وrequests عند unmount. لا تترك animation frame loop يعمل بينما panel مخفية.
16. استهدف rerender للمكوّن المتغير فقط؛ استخدم selectors أو snapshots مستقرة إذا كان store الحالي يدعمها. لا تضف `memo` عشوائيًا وتعتبره قياس أداء.
17. لا تنتظر نتيجة شبكة داخل مسار typing أو drag أو scroll. العمل الثقيل يكون في Rust أو worker مناسب، ونتائجه تصل بعقد قابل للإلغاء.

**ط.6 قياس الأداء**

أهداف قبول أولية تُقاس على جهاز اختبار موثق، وليست ادعاءات أداء منجزة:

- فتح Ctrl+K أو @ بعد الإحماء: p95 أقل من 100ms.
- تحديث العرض الناتج عن الكتابة: p95 أقل من 16ms على سيناريو الاختبار، دون long tasks متكررة فوق 50ms.
- اختفاء حالة التوليد بصريًا بعد Stop أقل من 100ms؛ استهدف تسوية الطلب/العملية محليًا خلال ثانية.
- عدم استهلاك نتيجة stale حتى لو أرسل المزوّد chunk بعد الإلغاء.
- لا تقفز selection أو scroll عند قبول hunk بعيد عن المؤشر.
- اختبر ملفًا من 10,000 سطر، ومحادثة طويلة من 1,000 رسالة، ومشروعًا من 10,000 ملف مصدر مؤهل.
- أبلغ عن حدود الجهاز والنتائج الحقيقية وأي تراجع؛ لا تدّعِ تحقيق 60 FPS من قراءة الكود فقط.

عرّف القياس بدقة: فتح composer من key event إلى أول frame يظهر فيه، وكلفة تحديث الكتابة المحلية منفصلة عن زمن العرض الكامل إلى الشاشة. قِس بعد الإحماء على build إنتاجي قدر الإمكان، وسجّل 100 عينة على الأقل عند الإبلاغ عن p95. لا تكتب رقم أداء بالتقدير البصري.

**ط.7 مصفوفة مراجعة الواجهة**

- الثيمات: Cosmic Dark وMoonlight Light وGalaxy، مع اختبار System عند تغيير إعداد النظام.
- النوافذ: نافذة Main ونافذة Agents؛ لا تعتمد على style أو provider يُحمّل في واحدة فقط.
- الحجم: الحد الأدنى الفعلي للنافذة حسب Tauri، ثم 1280×720 و1920×1080 حين تسمح الشاشة؛ سجّل viewport الفعلي بوحدات CSS pixels.
- DPI: Windows scaling عند 100% و150% إن أمكن. font scale: default وlarge وx-large. لا تخلط DPI مع إعداد حجم الخط.
- المحتوى: prompt عربي وإنجليزي مختلط، اسم ملف طويل، path بمسافات، diff كبير، قائمة فارغة، ورسالة خطأ طويلة.
- الحالات: idle/loading/generating/preview/error/conflict/disabled، واتصال بطيء أو منقطع.
- التفاعل: لوحة مفاتيح فقط، resize أثناء streaming، تبديل جلسات سريع، إغلاق الملف، وإغلاق النافذة أثناء الطلب.
- motion: الإعداد الطبيعي وreduced motion. التقدم يبقى مفهومًا في الحالتين دون الاعتماد على animation.

التقط صورًا للحالات الرئيسية لكل سطح جديد على dark وlight، مع حالة ضيقة وحالة خطأ. افحصها بنفسك قبل التسليم. إذا لم تستطع تشغيل التطبيق أو التقاط الصور، صرّح بأن الفحص البصري لم يُنفّذ؛ لا تضعه Pass.

«بدون مشاكل» تعني هنا عدم وجود عيب معروف في السيناريوهات التي اختُبرت: لا نص مقصوص، لا زر خارج النافذة، لا focus مفقود، لا scroll يقفز، لا تطبيق مكرر، ولا تعديل مستخدم ضائع. لا تقدّم ضمانًا مطلقًا بلا دليل.

#### ي. الاختبارات والتسليم

نفّذ المراحل بالترتيب. أنهِ كل مرحلة كمسار قابل للاستخدام من البداية للنهاية قبل توسيعها. لا تنشئ أزرارًا بلا backend، ولا تعرض نجاحًا أو تقدّمًا مصطنعًا.

اختبر الحالات الجوهرية للميزة التي تغيّرت:

- AI Tab: كتابة سريعة، out-of-order responses، تبديل tab/project، تداخل LSP، CRLF، indentation وsuffix overlap.
- Inline edit: تغيير النطاق أثناء streaming، إلغاء، خطأ شبكة، undo/redo، dirty buffer وAuto Save.
- Review: قبول جزء ورفض آخر، تكرار hunks متشابهة، تغيير يدوي لاحق، وتعديل الجلسات المتزامنة.
- Checkpoints: create/edit/delete، إعادة التشغيل، blob ناقص، فشل كتابة جزئي، conflict، وإعادة محاولة الاستعادة.
- Context: selection غير محفوظ، truncation، صورة مع نموذج غير داعم، وحفظ المسودة عبر Main وAgents.
- MCP: mock server، timeout، cancel، restart، تعارض أسماء الأدوات وserver failure.
- Indexing: تعديل وحذف وإعادة تسمية ملف، ignore متداخل، تبديل embedding model، وkeyword fallback.
- CLI: fixtures لأحداث الإصدارات المدعومة، مسار يحوي مسافات، exit غير ناجح، وإغلاق process tree.
- Terminal: PowerShell 5.1 و7، اقتباس ومسارات عربية، ومخرجات ANSI مجزأة.

**سيناريوهات قبول إلزامية بصيغة تنفيذية:**

1. **نتيجة قديمة:** ابدأ طلب Tab A، غيّر النص وابدأ B، ثم أعد نتيجة A بعد B. يجب أن يُعرض B فقط دون تغيير الملف.
2. **LSP وTab:** افتح قائمة LSP مع ghost text متاح. اضغط Tab مرة واحدة. يُقبل اختيار LSP فقط ولا يحدث إدراج ثانٍ.
3. **الكتابة أثناء التوليد:** اطلب Ctrl+K ثم غيّر حرفًا داخل النطاق. وصول النتيجة يؤدي إلى conflict، ويظل الحرف الذي كتبته محفوظًا.
4. **Auto Save:** فعّل Auto Save ثم شغّل Ctrl+K. النص الجزئي لا يصل للقرص. بعد Accept يعمل الحفظ حسب السياسة الحالية، ولا يفقد النص خارج النطاق.
5. **Undo:** اقبل اقتراحًا ثم اضغط Undo مرة. يختفي التعديل المقبول فقط وفق transaction group المتفق عليها. Redo يعيده مرة واحدة.
6. **Double click:** أرسل Accept مرتين للطلب نفسه. يُطبّق مرة واحدة، ولا يتكرر النص أو يتضاعف version بلا تعديل جديد.
7. **تبديل المشروع:** ابدأ طلبًا في مشروع A وانتقل إلى B قبل وصوله. لا يظهر اقتراح A ولا يتغير أي ملف في B.
8. **الإلغاء:** في أول توليد دون preview سابقة، اضغط Stop أثناء streaming ثم أرسل chunk وcompletion متأخرين من fixture. لا تعود الحالة إلى preview ولا يُطبّق نص. في follow-up لا تعود إلا إلى preview السابقة المحفوظة، دون عرض الناتج الملغى.
9. **المرفقات:** اكتب prompt وأرفق ملفًا، ثم بدّل الجلسة وعد إليها. تعود المسودة الصحيحة دون مرفقات الجلسة الأخرى.
10. **الواجهة الضيقة:** افتح Ctrl+K و@ عند حافة panel ضيقة مع x-large font. تبقى الإجراءات مرئية وقابلة للوصول دون scroll أفقي للتطبيق.
11. **قراءة المحادثة:** مرّر لأعلى أثناء streaming. لا يُسحب موضعك للأسفل؛ Jump to latest يعيدك عند الضغط فقط.
12. **الإغلاق المتكرر:** افتح وأغلق composer عشرين مرة ثم بدّل المشروع. لا تتضاعف handlers أو الطلبات، ولا تبقى timers/observers مرتبطة بالعناصر المغلقة.

نفّذ السيناريوهات ذات الصلة بالمرحلة الحالية، واترك البقية Pending حتى تنفيذ ميزاتها. اختبارات happy-dom لا تثبت أبعاد layout أو عمل WebView2/ConPTY؛ استخدمها للسلوك، واختبر العرض والربط داخل التطبيق الفعلي.

افحص baseline وسجّل الأخطاء الموجودة قبل عملك. بعد كل مرحلة شغّل الاختبارات المتعلقة بها، ثم فحص الأنواع والبناء عند تغيّر الواجهة:

```powershell
npm --prefix apps/selora-desktop/frontend run typecheck
npm --prefix apps/selora-desktop/frontend run test
npm --prefix apps/selora-desktop/frontend run build
```

شغّل `cargo check` و`cargo test` للحزم التي تغيّرت. مثال إذا مسّت المرحلة البروتوكول والوكيل وتطبيق سطح المكتب:

```powershell
cargo check -p selora-protocol -p selora-ai -p selora-desktop --locked
cargo test -p selora-protocol -p selora-ai -p selora-desktop --locked
```

تحقق من package names وقيود بيئة Windows قبل التنفيذ. الاختبارات الآلية لا تغني عن smoke test داخل Tauri للمحرر وTerminal والإلغاء والتبديل بين النوافذ.

عند اقتباس كود، احتفظ بإشعارات MIT وسجّل upstream path وcommit والتعديلات المحلية في ملف attribution مناسب. لا تنسخ ملفًا يستورد `vscode` إلى واجهة Tauri وتتوقع عمله.

سلّم لكل مرحلة:

1. ما أصبح يعمل فعليًا وكيف يستخدمه المستخدم.
2. الملفات المعدلة وعقود البروتوكول والتخزين الجديدة.
3. الكود المقتبس ومصدره، والكود الذي أُعيدت كتابته لتكييفه.
4. نتائج الاختبارات وقياسات الأداء الحقيقية.
5. القيود المتبقية ونقطة البداية المحددة للمرحلة التالية.
6. جدول subagents: Task ID، الدور، نطاق الملفات، نتيجة التسليم، ونتيجة مراجعة المنسّق.
7. نتيجة مراجعة UI: الحالات والأحجام والثيمات المفحوصة، وأماكن screenshots أو سبب عدم توفرها.

**بوابة انتهاء المرحلة:** العقد متوافق، مسار الميزة مربوط فعلًا، الاختبارات المتعلقة ناجحة، ومشاهد UI الحرجة مفحوصة. افصل فشل baseline عن أي regression أحدثته. لا تقل «جاهز بالكامل» إذا بقيت خطوة integration أو مراجعة بصرية لم تُنفّذ.

ابدأ الآن بقراءة التعليمات وحالة Git، ثم شغّل subagents الفحص الثلاثة بالتوازي. بعد تقاريرهم ثبّت عقد المرحلة الأولى وملكية الملفات، ثم أرسل بطاقات AI Tab المحددة. ادمجها واختبرها قبل موجة Ctrl+K. استمر بالمراحل التالية بعد نجاح تحقق المرحلة السابقة. إذا تجاوز العمل جلسة واحدة، سلّم مرحلة مكتملة مع سجل استئناف دقيق بدل تغييرات نصف مكتملة في جميع الميزات.

### نهاية البرومبت
