//! AS2 bytecode spike — static chrome-drawability analysis.
//!
//! Reads a SWF from the game P4K archive (or a local file) and analyses its
//! AVM1 action streams for Flash drawing-API calls.  The goal is to determine
//! whether the chrome shapes defined in BuildingBlocks SWFs can be statically
//! extracted without running an AVM1 interpreter.
//!
//! # Usage
//!
//! From P4K (two arguments):
//!   ```
//!   as2_spike <p4k_path> <swf_entry_path>
//!   ```
//!   e.g.
//!   ```
//!   as2_spike "$SC_DATA_P4K" \
//!     "Data\UI\BuildingBlocks\assets\SWF\BuildingBlocks_root.swf"
//!   ```
//!
//! From a local file (one argument):
//!   ```
//!   as2_spike <file.swf>
//!   ```
//!
//! # What it does
//!
//! 1. Walks every tag in the SWF (including tags inside `DefineSprite`).
//! 2. For each `DoAction` / `DoInitAction` block, parses the AVM1 action
//!    stream with `swf::avm1::read::Reader`.
//! 3. Reconstructs constant pools on-the-fly and follows `DefineFunction` /
//!    `DefineFunction2` bodies recursively.
//! 4. Detects Flash drawing-API call sites (`moveTo`, `lineTo`, `curveTo`,
//!    `beginFill`, `endFill`, `lineStyle`, `clear`, etc.) by looking for
//!    `CallMethod` / `GetMember` sequences where the method name string is
//!    a known drawing primitive.
//! 5. Classifies each call site as STATIC (all preceding Push arguments are
//!    literal numbers/strings with no register/variable references) or
//!    DYNAMIC (arguments involve registers, variables, arithmetic, or
//!    are resolved from a non-literal stack).
//! 6. Prints a per-function and per-class summary.

use std::collections::HashMap;
use std::io::Cursor;

use swf::avm1::read::Reader as Avm1Reader;
use swf::avm1::types::{Action, Value};
use swf::{Tag, UTF_8};

// ── Drawing-API method names we care about ───────────────────────────────────

const DRAW_METHODS: &[&str] = &[
    "moveTo",
    "lineTo",
    "curveTo",
    "beginFill",
    "beginGradientFill",
    "endFill",
    "lineStyle",
    "lineGradientStyle",
    "clear",
    "drawRect",
    "drawCircle",
    "drawEllipse",
    "drawRoundRect",
    "attachMovie",
    "createEmptyMovieClip",
];

const TEXT_METHODS: &[&str] = &[
    "defaultTextFormat",
    "setTextFormat",
    "textFormat",
    "htmlText",
    "text",
    "font",
    "size",
    "autoSize",
];

// ── Stack-value category for static analysis ────────────────────────────────

/// Coarse category of a value on the simulated AVM1 stack.
#[derive(Clone, Debug, PartialEq)]
enum StackVal {
    /// A literal numeric value (Int, Float, Double).
    LiteralNum(f64),
    /// A literal string constant (from Push or from constant pool).
    LiteralStr(String),
    /// Anything that requires runtime evaluation: register, variable,
    /// member access, arithmetic result, etc.
    Dynamic,
    /// Symbolic origin for unresolved but named values (e.g. variables/members).
    Symbol(String),
    /// Tracked object identity for InitObject/NewObject flows.
    ObjectRef(u32),
}

impl StackVal {
    fn is_literal(&self) -> bool {
        matches!(self, StackVal::LiteralNum(_) | StackVal::LiteralStr(_))
    }
}

// ── Per-call-site record ─────────────────────────────────────────────────────

#[derive(Debug)]
struct CallSite {
    method: String,
    /// True if all observed arguments were literals at parse time.
    all_static: bool,
    arg_count: usize,
    /// Sample of the top-N argument categories.
    arg_kinds: Vec<String>,
}

// ── Per-function analysis context ────────────────────────────────────────────

/// Analysis state for one AVM1 action-block pass.
struct FnCtx<'a> {
    /// The constant pool visible at this point in the action stream.
    constant_pool: Vec<String>,
    /// Simulated value stack (grows to the right / back = top).
    stack: Vec<StackVal>,
    /// Call sites found in this block.
    calls: Vec<CallSite>,
    /// Nested functions discovered (name → body bytes).
    nested_fns: Vec<(String, &'a [u8])>,
    /// Simple variable environment for SetVariable/GetVariable.
    variables: HashMap<String, StackVal>,
    /// Tracked object storage keyed by synthetic object id.
    objects: HashMap<u32, HashMap<String, StackVal>>,
    /// Register values tracked via StoreRegister and Push(Register).
    registers: HashMap<u8, StackVal>,
    /// Next synthetic object id.
    next_object_id: u32,
    /// Static member writes observed via SetMember (field, value).
    static_member_writes: Vec<(String, String)>,
    /// Text-related SetMember writes (field, value-kind/value) including dynamic.
    interesting_member_writes: Vec<(String, String)>,
    /// Focused capture for writes that may influence text size flow.
    font_size_related_writes: Vec<(String, String)>,
    /// Variable writes that may feed text-size logic.
    font_size_related_var_writes: Vec<(String, String)>,
    /// SWF version (needed to construct nested readers).
    version: u8,
}

impl<'a> FnCtx<'a> {
    fn new(version: u8) -> Self {
        Self {
            constant_pool: vec![],
            stack: vec![],
            calls: vec![],
            nested_fns: vec![],
            variables: HashMap::new(),
            objects: HashMap::new(),
            registers: HashMap::new(),
            next_object_id: 1,
            static_member_writes: vec![],
            interesting_member_writes: vec![],
            font_size_related_writes: vec![],
            font_size_related_var_writes: vec![],
            version,
        }
    }

    fn alloc_object(&mut self, seed: HashMap<String, StackVal>) -> u32 {
        let id = self.next_object_id;
        self.next_object_id += 1;
        self.objects.insert(id, seed);
        id
    }

    /// Resolve a `Value` to a `StackVal` given the current constant pool.
    fn resolve_value(&self, v: &Value<'_>) -> StackVal {
        match v {
            Value::Int(i) => StackVal::LiteralNum(*i as f64),
            Value::Float(f) => StackVal::LiteralNum(*f as f64),
            Value::Double(d) => StackVal::LiteralNum(*d),
            Value::Str(s) => StackVal::LiteralStr(s.to_string_lossy(UTF_8)),
            Value::ConstantPool(idx) => {
                let idx = *idx as usize;
                if idx < self.constant_pool.len() {
                    StackVal::LiteralStr(self.constant_pool[idx].clone())
                } else {
                    StackVal::Dynamic
                }
            }
            Value::Bool(_) => StackVal::Dynamic,
            Value::Undefined => StackVal::Dynamic,
            Value::Null => StackVal::Dynamic,
            Value::Register(reg) => self
                .registers
                .get(reg)
                .cloned()
                .unwrap_or(StackVal::Dynamic),
        }
    }

    /// Push a resolved value onto the simulated stack.
    fn push(&mut self, v: StackVal) {
        self.stack.push(v);
    }

    /// Pop the top value; return Dynamic if the stack is empty.
    fn pop(&mut self) -> StackVal {
        self.stack.pop().unwrap_or(StackVal::Dynamic)
    }

    /// Peek without consuming.
    fn peek(&self) -> StackVal {
        self.stack.last().cloned().unwrap_or(StackVal::Dynamic)
    }

    /// Process one action, updating the simulated stack and recording any
    /// drawing-API call sites found.
    fn process_action(&mut self, action: &Action<'a>) {
        match action {
            // ── Constant pool ──────────────────────────────────────────────
            Action::ConstantPool(pool) => {
                self.constant_pool = pool
                    .strings
                    .iter()
                    .map(|s| s.to_string_lossy(UTF_8))
                    .collect();
            }

            // ── Push ───────────────────────────────────────────────────────
            Action::Push(push) => {
                for v in &push.values {
                    let sv = self.resolve_value(v);
                    self.push(sv);
                }
            }

            // ── Pop ────────────────────────────────────────────────────────
            Action::Pop => {
                self.pop();
            }

            // ── Stack duplication ──────────────────────────────────────────
            Action::PushDuplicate => {
                let top = self.peek();
                self.push(top);
            }

            // ── Arithmetic / comparison — collapse to Dynamic ──────────────
            Action::Add
            | Action::Add2
            | Action::Subtract
            | Action::Multiply
            | Action::Divide
            | Action::Modulo
            | Action::StringAdd => {
                self.pop();
                self.pop();
                self.push(StackVal::Dynamic);
            }

            Action::Increment | Action::Decrement | Action::ToNumber | Action::ToString => {
                self.pop();
                self.push(StackVal::Dynamic);
            }

            Action::Equals
            | Action::Equals2
            | Action::StrictEquals
            | Action::Less
            | Action::Less2
            | Action::Greater
            | Action::And
            | Action::Or => {
                self.pop();
                self.pop();
                self.push(StackVal::Dynamic);
            }

            Action::Not => {
                self.pop();
                self.push(StackVal::Dynamic);
            }

            // ── Variable access — Dynamic ──────────────────────────────────
            Action::GetVariable => {
                let name = self.pop(); // variable name
                if let StackVal::LiteralStr(name) = name {
                    if let Some(value) = self.variables.get(name.as_str()) {
                        self.push(value.clone());
                    } else {
                        self.push(StackVal::Symbol(format!("var:{name}")));
                    }
                } else {
                    self.push(StackVal::Dynamic);
                }
            }

            Action::SetVariable => {
                let value = self.pop();
                let name = self.pop();
                if let StackVal::LiteralStr(name) = name {
                    let lower = name.to_ascii_lowercase();
                    if lower.contains("fontsize") || lower.contains("heading") || lower.contains("style") {
                        let rendered = match &value {
                            StackVal::LiteralNum(n) => format!("{n}"),
                            StackVal::LiteralStr(s) => format!("\"{s}\""),
                            StackVal::Symbol(s) => format!("SYM({s})"),
                            StackVal::ObjectRef(id) => format!("OBJ#{id}"),
                            StackVal::Dynamic => "DYN".to_string(),
                        };
                        self.font_size_related_var_writes.push((name.clone(), rendered));
                    }
                    self.variables.insert(name, value);
                }
            }

            // ── Member access — Dynamic ────────────────────────────────────
            Action::GetMember => {
                let member = self.pop(); // member name
                let object = self.pop(); // object
                match (object, member) {
                    (StackVal::ObjectRef(object_id), StackVal::LiteralStr(member_name)) => {
                        let value = self
                            .objects
                            .get(&object_id)
                            .and_then(|obj| obj.get(member_name.as_str()))
                            .cloned()
                            .unwrap_or(StackVal::Dynamic);
                        self.push(value);
                    }
                    (StackVal::Symbol(obj), StackVal::LiteralStr(member_name)) => {
                        self.push(StackVal::Symbol(format!("{obj}.{member_name}")));
                    }
                    (_, StackVal::LiteralStr(member_name)) => {
                        self.push(StackVal::Symbol(format!("member:{member_name}")));
                    }
                    _ => self.push(StackVal::Dynamic),
                }
            }

            Action::SetMember => {
                let value = self.pop();
                let member = self.pop();
                let object = self.pop(); // object

                if let (StackVal::ObjectRef(object_id), StackVal::LiteralStr(member_name)) =
                    (&object, &member)
                {
                    if let Some(obj) = self.objects.get_mut(object_id) {
                        obj.insert(member_name.clone(), value.clone());
                    }
                }

                if let StackVal::LiteralStr(member_name) = member {
                    let lower = member_name.to_ascii_lowercase();
                    let rendered = match &value {
                        StackVal::LiteralNum(n) => format!("{n}"),
                        StackVal::LiteralStr(s) => format!("\"{s}\""),
                        StackVal::Symbol(s) => format!("SYM({s})"),
                        StackVal::ObjectRef(id) => format!("OBJ#{id}"),
                        StackVal::Dynamic => "DYN".to_string(),
                    };

                    if lower.contains("fontsize")
                        || lower == "size"
                        || lower == "defaulttextformat"
                    {
                        self.font_size_related_writes
                            .push((member_name.clone(), rendered.clone()));
                    }

                    if matches!(
                        lower.as_str(),
                        "size" | "font" | "textformat" | "defaulttextformat" | "autosize"
                    ) {
                        self.interesting_member_writes.push((member_name.clone(), rendered.clone()));
                        if rendered != "DYN" {
                            self.static_member_writes.push((member_name, rendered));
                        }
                    }

                    if lower == "defaulttextformat" {
                        if let StackVal::ObjectRef(format_object_id) = value {
                            if let Some(format_obj) = self.objects.get(&format_object_id) {
                                if let Some(StackVal::LiteralNum(size)) = format_obj.get("size") {
                                    self.static_member_writes.push((
                                        "defaultTextFormat.size".to_string(),
                                        format!("{size}"),
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // ── CallMethod — primary detection point ───────────────────────
            //
            // AVM1 CallMethod stack layout (top → bottom):
            //   arg_count, argN..arg1, method_name, object
            //
            // Pop order: arg_count first, then args, then method_name, then object.
            Action::CallMethod => {
                let arg_count_val = self.pop();

                let arg_count: usize = match arg_count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => {
                        // Unknown arg count — drain up to 8 args conservatively.
                        8
                    }
                };

                // Pop the arguments BEFORE the method name.
                let mut arg_kinds = vec![];
                let mut all_static = true;
                for _ in 0..arg_count.min(32) {
                    let arg = self.pop();
                    if !arg.is_literal() {
                        all_static = false;
                    }
                    arg_kinds.push(match &arg {
                        StackVal::LiteralNum(n) => format!("{n}"),
                        StackVal::LiteralStr(s) => format!("\"{s}\""),
                        StackVal::Symbol(s) => format!("SYM({s})"),
                        StackVal::ObjectRef(id) => format!("OBJ#{id}"),
                        StackVal::Dynamic => "DYN".to_string(),
                    });
                }

                // Now pop method_name and object.
                let method_name_val = self.pop();
                let _object = self.pop();

                let method_name = match &method_name_val {
                    StackVal::LiteralStr(s) => s.clone(),
                    _ => String::from("<dynamic_method>"),
                };

                // Push the return value as Dynamic.
                self.push(StackVal::Dynamic);

                // Record if this is a drawing-API method.
                let is_draw = DRAW_METHODS
                    .iter()
                    .any(|&m| m.eq_ignore_ascii_case(&method_name));
                let is_text = TEXT_METHODS
                    .iter()
                    .any(|&m| m.eq_ignore_ascii_case(&method_name));
                if is_draw || is_text || method_name == "<dynamic_method>" {
                    self.calls.push(CallSite {
                        method: method_name,
                        all_static,
                        arg_count,
                        arg_kinds,
                    });
                }
            }

            // NewMethod is similar to CallMethod but for constructors.
            // Stack: arg_count, argN..arg1, method_name, object
            Action::NewMethod => {
                let arg_count_val = self.pop();
                let arg_count: usize = match arg_count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => 0,
                };
                for _ in 0..arg_count.min(32) {
                    self.pop();
                }
                let _method_name = self.pop();
                let _object = self.pop();
                self.push(StackVal::Dynamic);
            }

            // CallFunction — stack: arg_count, argN..arg1, fn_name
            Action::CallFunction => {
                let arg_count_val = self.pop();
                let arg_count: usize = match arg_count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => 0,
                };
                for _ in 0..arg_count.min(32) {
                    self.pop();
                }
                let _fn_name = self.pop();
                self.push(StackVal::Dynamic);
            }

            // NewObject — stack: arg_count, argN..arg1, class_name
            Action::NewObject => {
                let arg_count_val = self.pop();
                let arg_count: usize = match arg_count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => 0,
                };
                for _ in 0..arg_count.min(32) {
                    self.pop();
                }
                let class_name = self.pop();
                let mut seed = HashMap::new();
                if let StackVal::LiteralStr(class_name) = class_name {
                    seed.insert("__class__".to_string(), StackVal::LiteralStr(class_name));
                }
                let id = self.alloc_object(seed);
                self.push(StackVal::ObjectRef(id));
            }

            // ── DefineFunction / DefineFunction2 — recurse ─────────────────
            Action::DefineFunction(f) => {
                let name = f.name.to_string_lossy(UTF_8);
                self.nested_fns.push((name, f.actions));
            }

            Action::DefineFunction2(f) => {
                let name = f.name.to_string_lossy(UTF_8);
                self.nested_fns.push((name, f.actions));
            }

            // ── StoreRegister ──────────────────────────────────────────────
            Action::StoreRegister(reg) => {
                // Peek (doesn't consume the stack; the value is also stored).
                let value = self.peek();
                self.registers.insert(reg.register, value);
            }

            // ── Object/array init ──────────────────────────────────────────
            Action::InitObject => {
                let count_val = self.pop();
                let count: usize = match count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => 0,
                };
                let mut fields: Vec<(String, StackVal)> = Vec::new();
                for _ in 0..count {
                    let value = self.pop();
                    let key = self.pop();
                    if let StackVal::LiteralStr(key) = key {
                        fields.push((key, value));
                    }
                }
                let mut seed = HashMap::new();
                for (key, value) in fields.into_iter().rev() {
                    seed.insert(key, value);
                }
                let id = self.alloc_object(seed);
                self.push(StackVal::ObjectRef(id));
            }

            Action::InitArray => {
                let count_val = self.pop();
                let count: usize = match count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => 0,
                };
                for _ in 0..count {
                    self.pop();
                }
                self.push(StackVal::Dynamic);
            }

            // ── Return / End ───────────────────────────────────────────────
            Action::Return => {
                self.pop(); // return value
            }

            // ── Control flow ────────────────────────────────────────────────
            // If pops the condition; Jump has no stack effect.
            Action::If(_) => {
                self.pop(); // condition value
            }

            Action::Jump(_) => {
                // no stack effect
            }

            // ── StackSwap ──────────────────────────────────────────────────
            Action::StackSwap => {
                let a = self.pop();
                let b = self.pop();
                self.push(a);
                self.push(b);
            }

            // ── TypeOf / InstanceOf / CastOp ──────────────────────────────
            Action::TypeOf => {
                self.pop();
                self.push(StackVal::Dynamic);
            }

            Action::InstanceOf => {
                self.pop();
                self.pop();
                self.push(StackVal::Dynamic);
            }

            Action::CastOp => {
                self.pop(); // object
                self.pop(); // class
                self.push(StackVal::Dynamic);
            }

            // ── Extends — sets up prototype chain; stack: superclass, subclass ──
            Action::Extends => {
                self.pop();
                self.pop();
            }

            // ── ImplementsOp — pops count, then that many interfaces + 1 ──
            Action::ImplementsOp => {
                let count_val = self.pop();
                let count: usize = match count_val {
                    StackVal::LiteralNum(n) => n as usize,
                    _ => 0,
                };
                self.pop(); // constructor
                for _ in 0..count {
                    self.pop(); // interface
                }
            }

            // ── Throw — pops the exception object ─────────────────────────
            Action::Throw => {
                self.pop();
            }

            // ── Enumerate / Enumerate2 — unknown output count; flush stack ─
            // These are used in for..in loops; the pushed values are loop
            // keys and can't be known statically. Clear the stack to avoid
            // cascading mismatches.
            Action::Enumerate | Action::Enumerate2 => {
                self.pop(); // object/variable name
                // Push null sentinel + unknown keys — treat as one Dynamic
                // placeholder so the rest of the analysis isn't fully lost.
                self.push(StackVal::Dynamic);
            }

            // ── DefineLocal ────────────────────────────────────────────────
            Action::DefineLocal => {
                self.pop(); // value
                self.pop(); // name
            }

            Action::DefineLocal2 => {
                self.pop(); // name (value is undefined)
            }

            // ── GetProperty / SetProperty ──────────────────────────────────
            Action::GetProperty => {
                self.pop(); // property index
                self.pop(); // target
                self.push(StackVal::Dynamic);
            }

            Action::SetProperty => {
                self.pop(); // value
                self.pop(); // property index
                self.pop(); // target
            }

            // ── TargetPath ────────────────────────────────────────────────
            Action::TargetPath => {
                self.pop();
                self.push(StackVal::Dynamic);
            }

            // ── Everything else — treat as a black-box ─────────────────────
            _ => {}
        }
    }
}

// ── Recursive action-block analyser ─────────────────────────────────────────

/// Analysis result for one named function body.
#[derive(Debug)]
struct FnResult {
    name: String,
    call_sites: Vec<CallSite>,
    static_member_writes: Vec<(String, String)>,
    interesting_member_writes: Vec<(String, String)>,
    font_size_related_writes: Vec<(String, String)>,
    font_size_related_var_writes: Vec<(String, String)>,
    children: Vec<FnResult>,
}

/// Parse an AVM1 action-data byte slice and return call-site findings.
fn analyse_action_block<'a>(
    name: &str,
    data: &'a [u8],
    version: u8,
    parent_pool: &[String],
) -> FnResult {
    let mut ctx = FnCtx::new(version);
    // Inherit the outer constant pool so inner blocks can resolve references.
    ctx.constant_pool = parent_pool.to_vec();

    let mut reader = Avm1Reader::new(data, version);
    loop {
        match reader.read_action() {
            Ok(action) => {
                if matches!(action, Action::End) {
                    break;
                }
                ctx.process_action(&action);
            }
            Err(_) => break,
        }
    }

    // Recursively analyse nested functions.
    let inherited_pool = ctx.constant_pool.clone();
    let nested = ctx.nested_fns.drain(..).collect::<Vec<_>>();
    let children = nested
        .into_iter()
        .map(|(child_name, body)| analyse_action_block(&child_name, body, version, &inherited_pool))
        .collect();

    FnResult {
        name: name.to_string(),
        call_sites: ctx.calls,
        static_member_writes: ctx.static_member_writes,
        interesting_member_writes: ctx.interesting_member_writes,
        font_size_related_writes: ctx.font_size_related_writes,
        font_size_related_var_writes: ctx.font_size_related_var_writes,
        children,
    }
}

// ── SWF tag walker ───────────────────────────────────────────────────────────

/// Summary statistics accumulated across an entire SWF.
#[derive(Default, Debug)]
struct SwfSummary {
    /// Total DoAction + DoInitAction blocks found.
    action_blocks: usize,
    /// All function analysis results (top-level blocks and their nested fns).
    roots: Vec<FnResult>,
    /// Exports found: name → character id.
    exports: HashMap<String, u16>,
    /// SWF version.
    version: u8,
    /// All constant-pool and Push-string values seen across ALL action blocks.
    /// Used as a ground-truth check: if draw method names appear here, the
    /// SWF contains drawing-API calls regardless of stack simulation accuracy.
    all_string_constants: Vec<String>,
    /// Raw CallMethod count (irrespective of method name resolution).
    raw_callmethod_count: usize,
}

impl SwfSummary {
    /// Count drawing-API call sites across the entire tree.
    fn draw_call_count(&self) -> (usize, usize) {
        let mut total = 0usize;
        let mut dynamic = 0usize;
        for root in &self.roots {
            visit_calls(root, &mut |c| {
                total += 1;
                if !c.all_static {
                    dynamic += 1;
                }
            });
        }
        (total, dynamic)
    }
}

fn visit_calls<F: FnMut(&CallSite)>(result: &FnResult, f: &mut F) {
    for c in &result.call_sites {
        f(c);
    }
    for child in &result.children {
        visit_calls(child, f);
    }
}

/// Scan an action-data block and collect all string literals from
/// `ConstantPool` and `Push` actions, plus count raw `CallMethod` occurrences.
///
/// This is a ground-truth pass that does NOT try to track the stack — it just
/// harvests every string the SWF ever uses and counts every call instruction.
/// If drawing-API names appear here, the SWF provably uses the Flash drawing API.
fn scan_string_constants(data: &[u8], version: u8, out: &mut Vec<String>, call_count: &mut usize) {
    let mut reader = Avm1Reader::new(data, version);
    loop {
        match reader.read_action() {
            Ok(action) => match &action {
                Action::End => break,
                Action::ConstantPool(pool) => {
                    for s in &pool.strings {
                        out.push(s.to_string_lossy(UTF_8));
                    }
                }
                Action::Push(push) => {
                    for v in &push.values {
                        if let Value::Str(s) = v {
                            out.push(s.to_string_lossy(UTF_8));
                        }
                    }
                }
                Action::CallMethod => {
                    *call_count += 1;
                }
                // Recurse into function bodies.
                Action::DefineFunction(f) => {
                    scan_string_constants(f.actions, version, out, call_count);
                }
                Action::DefineFunction2(f) => {
                    scan_string_constants(f.actions, version, out, call_count);
                }
                _ => {}
            },
            Err(_) => break,
        }
    }
}

/// Walk all tags in a `Swf<'_>` and accumulate analysis into `summary`.
fn walk_tags(tags: &[Tag<'_>], summary: &mut SwfSummary) {
    for tag in tags {
        match tag {
            Tag::DoAction(data) => {
                summary.action_blocks += 1;
                let result = analyse_action_block(
                    &format!("DoAction#{}", summary.action_blocks),
                    data,
                    summary.version,
                    &[],
                );
                scan_string_constants(
                    data,
                    summary.version,
                    &mut summary.all_string_constants,
                    &mut summary.raw_callmethod_count,
                );
                summary.roots.push(result);
            }

            Tag::DoInitAction { action_data, .. } => {
                summary.action_blocks += 1;
                let result = analyse_action_block(
                    &format!("DoInitAction#{}", summary.action_blocks),
                    action_data,
                    summary.version,
                    &[],
                );
                scan_string_constants(
                    action_data,
                    summary.version,
                    &mut summary.all_string_constants,
                    &mut summary.raw_callmethod_count,
                );
                summary.roots.push(result);
            }

            Tag::DefineSprite(sprite) => {
                // Recurse into the sprite's tag list.
                walk_tags(&sprite.tags, summary);
            }

            Tag::ExportAssets(exports) => {
                for asset in exports.iter() {
                    summary
                        .exports
                        .insert(asset.name.to_string_lossy(UTF_8), asset.id);
                }
            }

            // DoAbc / DoAbc2 are AVM2 (AS3) — not present in AS2 SWFs.
            Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                eprintln!(
                    "[warn] Found AVM2 (DoAbc) tag — this SWF uses AS3, not AS2.  \
                     Analysis will be incomplete."
                );
            }

            _ => {}
        }
    }
}

// ── Pretty-print helpers ─────────────────────────────────────────────────────

fn print_fn_result(result: &FnResult, depth: usize) {
    let indent = "  ".repeat(depth);

    // Only print functions that have draw calls or interesting children.
    let (total, dyn_count) = {
        let mut t = 0;
        let mut d = 0;
        visit_calls(result, &mut |c| {
            t += 1;
            if !c.all_static {
                d += 1;
            }
        });
        (t, d)
    };

    let has_text_write_signals = !result.static_member_writes.is_empty()
        || !result.interesting_member_writes.is_empty()
        || !result.font_size_related_writes.is_empty();

    if total == 0 && result.children.is_empty() && !has_text_write_signals {
        return;
    }

    let fn_label = if result.name.is_empty() {
        "<anonymous>".to_string()
    } else {
        result.name.clone()
    };

    if total > 0 {
        println!(
            "{indent}fn {fn_label}: {} draw call(s), {} static, {} dynamic",
            total,
            total - dyn_count,
            dyn_count
        );
        for c in &result.call_sites {
            let kind = if c.all_static { "STATIC" } else { "DYNAMIC" };
            println!("{indent}  [{kind}] .{}() args={}", c.method, c.arg_count);
            if !c.arg_kinds.is_empty() {
                // Print the arg list (reversed: innermost pushed last = leftmost arg).
                let args: Vec<&str> = c.arg_kinds.iter().rev().take(8).map(|s| s.as_str()).collect();
                println!("{indent}          stack args: {}", args.join(", "));
            }
        }
    } else if !result.children.is_empty() || has_text_write_signals {
        println!("{indent}fn {fn_label}: (no draw calls, has nested/text signals)");
    }

    if !result.static_member_writes.is_empty() {
        println!(
            "{indent}  static text member writes: {}",
            result.static_member_writes.len()
        );
        for (member, value) in result.static_member_writes.iter().take(16) {
            println!("{indent}    {} = {}", member, value);
        }
    }

    if !result.interesting_member_writes.is_empty() {
        println!(
            "{indent}  interesting text member writes (incl. dynamic): {}",
            result.interesting_member_writes.len()
        );
        for (member, value) in result.interesting_member_writes.iter().take(16) {
            println!("{indent}    {} := {}", member, value);
        }
    }

    if !result.font_size_related_writes.is_empty() {
        println!(
            "{indent}  font-size related writes: {}",
            result.font_size_related_writes.len()
        );
        for (member, value) in result.font_size_related_writes.iter().take(24) {
            println!("{indent}    {} ~~ {}", member, value);
        }
    }

    if !result.font_size_related_var_writes.is_empty() {
        println!(
            "{indent}  font-size related variable writes: {}",
            result.font_size_related_var_writes.len()
        );
        for (name, value) in result.font_size_related_var_writes.iter().take(24) {
            println!("{indent}    var {} ~~ {}", name, value);
        }
    }

    for child in &result.children {
        print_fn_result(child, depth + 1);
    }
}

// ── Constant-pool string dump ────────────────────────────────────────────────

fn collect_all_strings(result: &FnResult) -> Vec<String> {
    // Re-parse to get the constant pool (we'd need to thread it through;
    // instead, just collect all LiteralStr arguments from call sites).
    let mut strings = vec![];
    for c in &result.call_sites {
        for a in &c.arg_kinds {
            if a.starts_with('"') {
                strings.push(a.trim_matches('"').to_string());
            }
        }
    }
    for child in &result.children {
        strings.extend(collect_all_strings(child));
    }
    strings
}

// ── P4K extraction helper ────────────────────────────────────────────────────

fn read_swf_bytes(args: &[String]) -> anyhow::Result<(Vec<u8>, String)> {
    match args.len() {
        1 => {
            // Single argument: path to a SWF file on disk.
            let path = &args[0];
            let bytes = std::fs::read(path)?;
            Ok((bytes, path.clone()))
        }
        2 => {
            // Two arguments: P4K path + SWF entry path (backslash-separated).
            let p4k_path = &args[0];
            let swf_entry = &args[1];
            let p4k = starbreaker_p4k::MappedP4k::open(p4k_path)?;
            let bytes = p4k.read_file(swf_entry)?;
            Ok((bytes, swf_entry.clone()))
        }
        _ => {
            anyhow::bail!(
                "Usage:\n  as2_spike <file.swf>\n  as2_spike <p4k_path> <swf_entry_path>"
            );
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (bytes, label) = read_swf_bytes(&args)?;

    println!("=== AS2 Spike: {label} ===");
    println!("Raw bytes: {}", bytes.len());

    // Decompress and parse.
    let swf_buf = swf::decompress_swf(Cursor::new(&bytes))?;
    let version = swf_buf.header.version();
    let swf = swf::parse_swf(&swf_buf)?;

    println!("SWF version: {version}");
    println!(
        "Frame size: {}x{}",
        swf.header.stage_size().x_max - swf.header.stage_size().x_min,
        swf.header.stage_size().y_max - swf.header.stage_size().y_min
    );
    println!("Tags: {}", swf.tags.len());

    let mut summary = SwfSummary {
        version,
        ..Default::default()
    };
    walk_tags(&swf.tags, &mut summary);

    println!("\n--- Exports ({}) ---", summary.exports.len());
    let mut export_names: Vec<_> = summary.exports.keys().collect();
    export_names.sort();
    for name in &export_names {
        println!("  {name}");
    }

    println!("\n--- Action blocks: {} ---", summary.action_blocks);

    let (total_draw, dyn_draw) = summary.draw_call_count();
    println!("Drawing-API call sites found: {total_draw}");
    println!(
        "  STATIC (all literal args): {}",
        total_draw - dyn_draw
    );
    println!("  DYNAMIC (runtime-dependent): {dyn_draw}");

    if total_draw > 0 {
        println!("\n--- Draw-call detail (functions with draw calls) ---");
        for root in &summary.roots {
            print_fn_result(root, 0);
        }
    }

    // Chrome-shape search: look for functions whose name or enclosing context
    // references known chrome shape names.
    let chrome_keywords = [
        "chevron",
        "pagegreeble",
        "notargetbox",
        "separator",
        "footer",
        "annunciator",
        "radar",
        "chiclet",
        "target_status",
        "targetstatus",
        "shape_",
        "drawshape",
        "_rendershape",
        "initwidget",
        "redraw",
        "draw",
    ];

    println!("\n--- Chrome-shape function search ---");
    let mut found_chrome = false;
    for root in &summary.roots {
        search_chrome(root, &chrome_keywords, 0, &mut found_chrome);
    }
    if !found_chrome {
        println!("  (no functions with chrome-related names found)");
    }

    // Print all method-name strings seen to help identify framework calls.
    println!("\n--- All drawing-method names seen ---");
    let mut method_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for root in &summary.roots {
        collect_method_names(root, &mut method_names);
    }
    let mut sorted_methods: Vec<_> = method_names.into_iter().collect();
    sorted_methods.sort();
    for m in &sorted_methods {
        println!("  {m}");
    }

    // Ground-truth string-constant scan.
    println!("\n--- Ground-truth string constant scan ---");
    println!("Raw CallMethod count: {}", summary.raw_callmethod_count);
    let mut draw_strings: Vec<String> = summary
        .all_string_constants
        .iter()
        .filter(|s| {
            DRAW_METHODS
                .iter()
                .any(|&m| m.eq_ignore_ascii_case(s.as_str()))
        })
        .cloned()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    draw_strings.sort();
    if draw_strings.is_empty() {
        println!("  No draw-API method names found in constant pool / Push strings.");
        println!("  CONCLUSION: This SWF does NOT call the Flash drawing API directly.");
    } else {
        println!("  Draw API strings found in bytecode constants:");
        for s in &draw_strings {
            let count = summary
                .all_string_constants
                .iter()
                .filter(|x| x.as_str().eq_ignore_ascii_case(s))
                .count();
            println!("    '{s}' × {count}");
        }
        println!("  CONCLUSION: This SWF DOES contain Flash drawing-API method names.");
    }

    // Print all unique string constants that look like draw/widget/shape names.
    let interesting_patterns = [
        "moveto", "lineto", "curveto", "beginfill", "endfill",
        "linestyle", "clear", "drawrect", "drawcircle",
        "chevron", "greeble", "targetbox", "separator", "annunciator",
        "radar", "shape_", "draw", "render", "_canvas", "canvas",
    ];
    let mut interesting: Vec<String> = summary
        .all_string_constants
        .iter()
        .filter(|s| {
            let lc = s.to_lowercase();
            interesting_patterns.iter().any(|p| lc.contains(p))
        })
        .cloned()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    interesting.sort();
    if !interesting.is_empty() {
        println!("\n  Interesting strings (draw/shape/canvas-related) in constants:");
        for s in &interesting {
            println!("    {s}");
        }
    }

    Ok(())
}

fn search_chrome(result: &FnResult, keywords: &[&str], depth: usize, found: &mut bool) {
    let name_lc = result.name.to_lowercase();
    let is_chrome = keywords.iter().any(|k| name_lc.contains(k));

    if is_chrome {
        *found = true;
        let indent = "  ".repeat(depth);
        let (total, dyn_count) = {
            let mut t = 0;
            let mut d = 0;
            visit_calls(result, &mut |c| {
                t += 1;
                if !c.all_static {
                    d += 1;
                }
            });
            (t, d)
        };
        println!(
            "{indent}CHROME fn '{}': {} draw calls ({} static, {} dynamic)",
            result.name,
            total,
            total - dyn_count,
            dyn_count
        );
        for c in &result.call_sites {
            let kind = if c.all_static { "STATIC" } else { "DYN" };
            let args: Vec<_> = c.arg_kinds.iter().rev().take(8).collect();
            println!(
                "{indent}  [{kind}] .{}({}) ",
                c.method,
                args.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    for child in &result.children {
        search_chrome(child, keywords, depth + 1, found);
    }
}

fn collect_method_names(result: &FnResult, out: &mut std::collections::HashSet<String>) {
    for c in &result.call_sites {
        out.insert(c.method.clone());
    }
    for child in &result.children {
        collect_method_names(child, out);
    }
}
