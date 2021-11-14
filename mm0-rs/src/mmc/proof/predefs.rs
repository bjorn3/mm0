use mm0_util::{SortId, TermId, ThmId};

use crate::{DeclKey, Environment};

fn mk_array<A, const N: usize>(mut f: impl FnMut(usize) -> A) -> [A; N] {
  let mut i = 0_usize;
  [(); N].map(|_| { let a = f(i); i += 1; a })
}

fn mk_remap<'a, A, const N: usize>(
  arr: &'a [A; N],
  mut f: impl FnMut(usize, &'a A) -> A
) -> [A; N] {
  let mut i = 0_usize;
  [(); N].map(|_| { let a = f(i, &arr[i]); i += 1; a })
}

fn get_sort(env: &Environment, s: &[u8]) -> SortId {
  if_chain! {
    if let Some(&a) = env.atoms.get(s);
    if let Some(sort) = env.data[a].sort;
    then { sort }
    else { panic!("sort {} not found", std::str::from_utf8(s).expect("utf8")) }
  }
}
fn get_term(env: &Environment, s: &[u8]) -> TermId {
  if_chain! {
    if let Some(&a) = env.atoms.get(s);
    if let Some(DeclKey::Term(t)) = env.data[a].decl;
    then { t }
    else { panic!("term {} not found", std::str::from_utf8(s).expect("utf8")) }
  }
}
fn get_thm(env: &Environment, s: &[u8]) -> ThmId {
  if_chain! {
    if let Some(&a) = env.atoms.get(s);
    if let Some(DeclKey::Thm(t)) = env.data[a].decl;
    then { t }
    else { panic!("thm {} not found", std::str::from_utf8(s).expect("utf8")) }
  }
}

macro_rules! make_predefs {
  (@ty $ty:tt $n:expr, $($ns:expr,)*) => {[make_predefs!(@ty $ty $($ns,)*); $n]};
  (@ty $ty:ident) => {$ty};
  (@new $ty:tt $env:expr, ($i:ident, $($is:ident,)*) $cond:tt $e:expr) => {
    mk_array(|$i| make_predefs!(@new $ty $env, ($($is,)*) $cond $e))
  };
  (@new $ty:ident $env:expr, () ($cond:expr) $e:expr) => {
    if $cond { make_predefs!(@new $ty $env, () () $e) } else { $ty(0) }
  };
  (@new AtomId $env:expr, () () $e:expr) => { $env.get_atom($e) };
  (@new SortId $env:expr, () () $e:expr) => { get_sort($env, $e) };
  (@new TermId $env:expr, () () $e:expr) => { get_term($env, $e) };
  (@new ThmId $env:expr, () () $e:expr) => { get_thm($env, $e) };
  (@remap $ty:tt $self:expr, $r:expr, ($i:ident, $($is:ident,)*) $cond:tt) => {
    mk_remap($self, |$i, this| make_predefs!(@remap $ty this, $r, ($($is,)*) $cond))
  };
  (@remap $ty:ident $self:expr, $r:expr, () ($cond:expr)) => {
    if $cond { make_predefs!(@remap $ty $self, $r, () ()) } else { $ty(0) }
  };
  (@remap $ty:ident $self:expr, $r:expr, () ()) => {$self.remap($r)};
  {$($(#[$attr:meta])* $x:ident $([$i:ident: $n:expr])*:
      $ty:tt $(if $cond:expr)? => $e:expr;)*} => {
    /// A predef is a name of an external constant, defined in `compiler.mm1` and required
    /// for proof output.
    #[derive(Copy, Clone, Debug, EnvDebug, Serialize, Deserialize)]
    pub(crate) struct Predefs {
      $(
        #[allow(non_snake_case)] $(#[$attr])*
        pub(crate) $x: make_predefs!(@ty $ty $($n,)*),
      )*
    }
    #[cfg(feature = "memory")] mm0_deepsize::deep_size_0!(Predefs);

    impl crate::Remap for Predefs {
      type Target = Self;
      fn remap(&self, r: &mut crate::Remapper) -> Self {
        Self { $($x: make_predefs!(@remap $ty &self.$x, r, ($($i,)*) ($($cond)?))),* }
      }
    }

    impl Predefs {
      /// Construct a `Predefs` from an environment.
      pub(crate) fn new(env: &mut crate::Environment) -> Self {
        #[allow(clippy::string_lit_as_bytes)]
        Self { $($x: make_predefs!(@new $ty env, ($($i,)*) ($($cond)?) $e.as_bytes())),* }
      }
    }
  };
}

make_predefs! {
  /// `$F.$: wff`
  fal: TermId => "fal";
  /// `$T.$: wff`
  tru: TermId => "tru";
  /// `eq: nat > nat > wff`
  eq: TermId => "eq";
  /// `eq: nat > nat > wff`
  ne: TermId => "ne";
  /// `strict sort set;`
  set: SortId => "set";
  /// `0: nat`
  d0: TermId => "d0";
  /// `suc: nat > nat`
  suc: TermId => "suc";
  /// `add: nat > nat > nat`
  add: TermId => "add";
  /// `mul: nat > nat > nat`
  mul: TermId => "mul";
  /// `le: nat > nat > wff`
  le: TermId => "le";
  /// `lt: nat > nat > wff`
  lt: TermId => "lt";
  /// `ltlei (h: $ a < b $): $ a <= b $`
  ltlei: ThmId => "ltlei";
  /// `ltnei (h: $ a < b $): $ a != b $`
  ltnei: ThmId => "ltnei";
  /// `ltneri (h: $ a < b $): $ b != a $`
  ltneri: ThmId => "ltneri";
  /// `leid: $ a <= a $`
  leid: ThmId => "leid";
  /// `znsub: `
  znsub: TermId => "znsub";
  /// `pr: nat > nat > nat`
  pr: TermId => "pr";
  /// `cons: nat > nat > nat`
  cons: TermId => "cons";
  /// `strict sort string;`
  string: SortId => "string";
  /// `sadd: string > string > string`
  sadd: TermId => "sadd";
  /// `sadd: char > string > string`
  scons: TermId => "scons";
  /// `s0: string`
  s0: TermId => "s0";
  /// `s1: char > string`
  s1: TermId => "s1";
  /// `d[0..=f]: nat`
  dn[i: 17]: TermId => format!("d{}", i);
  /// `x[0..=f]: hex`
  xn[i: 16]: TermId => format!("x{:x}", i);
  /// `h2n: hex > nat`
  h2n: TermId => "h2n";
  /// `hex: nat > hex > nat`
  hex: TermId => "hex";
  /// `ch: hex > hex > char`
  ch: TermId => "ch";
  /// `c2n: char > nat`
  c2n: TermId => "c2n";
  /// `s2n: string > nat`
  s2n: TermId => "s2n";
  /// `h2n[0..=f]: x[i] = d[i]`
  h2nn[i: 16]: ThmId => format!("h2n{:x}", i);
  /// `decsucxf (a b c): suc a = b > suc (hex a xf) (hex b x0)`
  decsucxf: ThmId => "decsucxf";
  /// `decsucx (a b c): suc b = c > suc (hex a b) (hex a c)`
  decsucx: ThmId => "decsucx";
  /// `decsuc[0..f]: suc x[a] = hex(a+1)`
  decsucn[i: 16]: ThmId => format!("decsuc{:x}", i);
  /// `declt[0..f][0..=f]: x[a] < x[b]` for `a < b`
  decltn[a: 15][b: 16]: ThmId if a < b => format!("declt{:x}{:x}", a, b);
  /// `decltx1 (h: $ a < c $): $ a :x b < c :x d $`
  decltx1: ThmId => "decltx1";
  /// `decltx2 (h: $ b < c $): $ a :x b < a :x c $`
  decltx2: ThmId => "decltx2";
  /// `declt0x (h: $ x0 < b $): $ h2n a < b :x c $`
  declt0x: ThmId => "declt0x";
  /// `decadd[0..=f][0..=f]: x[a] + x[b] = hex(a+b)`
  decaddn[a: 16][b: 16]: ThmId => format!("decadd{:x}{:x}", a, b);
  /// `decadc[0..=f][0..=f]: suc (x[a] + x[b]) = hex(a+b+1)`
  decadcn[a: 16][b: 16]: ThmId => format!("decadc{:x}{:x}", a, b);

  // Theorems to compute `a + b = c` and `suc (a + b) = c`
  add_xx0: ThmId => "add_xx0";
  add_xx1: ThmId => "add_xx1";
  add_0x0: ThmId => "add_0x0";
  add_0x1: ThmId => "add_0x1";
  add_x00: ThmId => "add_x00";
  add_x01: ThmId => "add_x01";
  adc_xx0: ThmId => "adc_xx0";
  adc_xx1: ThmId => "adc_xx1";
  adc_0x0: ThmId => "adc_0x0";
  adc_0x1: ThmId => "adc_0x1";
  adc_x00: ThmId => "adc_x00";
  adc_x01: ThmId => "adc_x01";

  /// `bit: nat > nat > nat`
  bit: TermId => "bit";
  xbit[n: 16][i: 4]: ThmId => format!("xbit{:x}{:x}", n, i);

  /// `wSz8 (have_rex: wff): nat`
  wSz8: TermId => "wSz8";
  /// `wSz32: nat`
  wSz32: TermId => "wSz32";
  /// `wSz64: nat`
  wSz64: TermId => "wSz64";

  opSize: TermId => "opSize";
  opSize_64: ThmId => "opSize_64";
  opSize_32: ThmId => "opSize_32";
  opSize_8: ThmId => "opSize_8";

  opSizeW: TermId => "opSizeW";
  opSizeW_0: ThmId => "opSizeW_0";
  opSizeW_S: ThmId => "opSizeW_S";

  REX[i: 4]: TermId => ["REX_B", "REX_X", "REX_R", "REX_W"][i];
  REX_0[i: 4]: ThmId => ["REX_B_0", "REX_X_0", "REX_R_0", "REX_W_0"][i];
  REX_Si[i: 4]: ThmId => ["REX_B_Si", "REX_X_Si", "REX_R_Si", "REX_W_Si"][i];

  base_RIP: TermId => "base_RIP";
  base_reg: TermId => "base_reg";

  unopInc: TermId => "unopInc";
  unopDec: TermId => "unopDec";
  unopNot: TermId => "unopNot";
  unopNeg: TermId => "unopNeg";

  padn[i: 16]: TermId => format!("_x00x{:x}", i);

  /// `assemble (s: string) (x y: nat) (P: set): wff`
  assemble: TermId => "assemble";
  /// `assembleA (A B: set): set`
  assembleA: TermId => "assembleA";
  assembleA_I: ThmId => "assembleA_I";

  /// `localAssemble (p: nat) (s: string) (x y: nat) (P: set): wff`
  localAssemble: TermId => "localAssemble";
  /// `localAssembleA (A B: set): set`
  localAssembleA: TermId => "localAssembleA";
  localAssembleA_I: ThmId => "localAssembleA_I";

  /// `localAssemble0 (p: nat) (x: nat) (P: set): wff`
  localAssemble0: TermId => "localAssemble0";
  localAssemble0_l: ThmId => "localAssemble0_l";
  localAssemble0_r: ThmId => "localAssemble0_r";
  localAssemble0_A: ThmId => "localAssemble0_A";

  /// `asmAt (n: nat) (A: set): set`
  asmAt: TermId => "asmAt";
  asmAtI: ThmId => "asmAtI";
  asmAt0: ThmId => "asmAt0";

  /// `asmEntry (p: nat) (A: set): set`
  asmEntry: TermId => "asmEntry";
  asmEntryI: ThmId => "asmEntryI";
  asmEntry0: ThmId => "asmEntry0";

  /// `asmProc (n: nat) (A: set): set`
  asmProc: TermId => "asmProc";
  asmProcI: ThmId => "asmProcI";
  assemble_pad: ThmId => "assemble_pad";

  /// `assembled (c: string) (P: set): wff`
  assembled: TermId => "assembled";
  assembledI: ThmId => "assembledI";
  assembled_l: ThmId => "assembled_l";
  assembled_r: ThmId => "assembled_r";

  /// `strlen (s: string) (n: nat): wff`
  strlen: TermId => "strlen";
  strlenn[i: 16]: ThmId => format!("strlen{:x}", i);
  strlen_padn[i: 16]: ThmId if i != 0 => format!("strlen_x00x{:x}", i);

  /// `parseInst (p ip: nat) (s: string) (I: set): wff`
  parseInst: TermId => "parseInst";
  parseInstE: ThmId => "parseInstE";
  parseInst01: ThmId => "parseInst01";
  parseInst11: ThmId => "parseInst11";
  parseInst00: ThmId => "parseInst00";
  parseInst10: ThmId => "parseInst10";

  /// `IRM_reg (reg: hex): nat`
  IRM_reg: TermId => "IRM_reg";
  /// `IRM_mem (si base off: nat): nat`
  IRM_mem: TermId => "IRM_mem";
  /// `IRM_imm32 (imm: nat): nat`
  IRM_imm32: TermId => "IRM_imm32";
  /// `IRM_imm64 (imm: nat): nat`
  IRM_imm64: TermId => "IRM_imm64";

  /// `isU64 (n: nat): wff`
  isU64: TermId => "isU64";
  isU64n[i: 16]: ThmId => format!("isU64_{:x}", i);

  /// `parseUBytes (k n: nat) (s: string): wff`
  parseUBytes: TermId => "parseUBytes";

  /// `parseIBytesPos (k n: nat) (s: string): wff`
  parseIBytesPos: TermId => "parseIBytesPos";
  parseIBytesPosS: ThmId => "parseIBytesPosS";
  parseIBytesPosS2: ThmId => "parseIBytesPosS2";
  parseIBytesPosS1: ThmId => "parseIBytesPosS1";
  parseIBytesPos02: ThmId => "parseIBytesPos02";
  parseIBytesPos01: ThmId => "parseIBytesPos01";

  /// `parseIBytesNeg (k n: nat) (s: string): wff`
  parseIBytesNeg: TermId => "parseIBytesNeg";
  parseIBytesNegS: ThmId => "parseIBytesNegS";
  parseIBytesNegS2: ThmId => "parseIBytesNegS2";
  parseIBytesNegS1: ThmId => "parseIBytesNegS1";
  parseIBytesNegS0: ThmId => "parseIBytesNegS0";
  parseIBytesNeg02: ThmId => "parseIBytesNeg02";
  parseIBytesNeg01: ThmId => "parseIBytesNeg01";

  /// `posZ: nat > nat` (the `nat > int` injection)
  posZ: TermId => "posZ";
  /// `negZ: nat > nat` (semantically `nat > int`, maps `n: nat` to `-n-1: int` )
  negZ: TermId => "negZ";
  znsub_posZ: ThmId => "znsub_posZ";
  znsub_negZ: ThmId => "znsub_negZ";

  /// `parseImmN (k imm: nat) (s: string): wff`
  parseImmN: TermId => "parseImmN";
  parseImmN_pos: ThmId => "parseImmN_pos";
  parseImmN_neg: ThmId => "parseImmN_neg";

  /// `parseImm8 (imm: nat) (s: string): wff`
  parseImm8: TermId => "parseImm8";
  parseImm8_I: ThmId => "parseImm8_I";
  /// `parseImm32 (imm: nat) (s: string): wff`
  parseImm32: TermId => "parseImm32";
  parseImm32_I: ThmId => "parseImm32_I";
  /// `parseImm64 (imm: nat) (s: string): wff`
  parseImm64: TermId => "parseImm64";
  parseImm64_I: ThmId => "parseImm64_I";
  /// `parseImm8S (imm: nat) (s s2: string): wff`
  parseImm8S: TermId => "parseImm8S";
  parseImm8S_I: ThmId => "parseImm8S_I";
  /// `parseImm32S (imm: nat) (s s2: string): wff`
  parseImm32S: TermId => "parseImm32S";
  parseImm32S_I: ThmId => "parseImm32S_I";

  /// `parseImm (sz imm: nat) (s: string): wff`
  parseImm: TermId => "parseImm";
  parseImm_8: ThmId => "parseImm_8";
  parseImm_32: ThmId => "parseImm_32";
  parseImm_64: ThmId => "parseImm_64";

  splitBits[i: 5]: TermId => format!("splitBits{}", SPLIT_BITS_NAMES[i]);
  splitBitsn[i: 5][n: 16]: ThmId => format!("splitBits{}_{:x}", SPLIT_BITS_NAMES[i], n);

  /// `parseDisplacement (mod q: nat) (l l2: string): wff`
  parseDisplacement: TermId => "parseDisplacement";
  parseDisplacement_0: ThmId => "parseDisplacement_0";
  parseDisplacement_8: ThmId => "parseDisplacement_8";
  parseDisplacement_32: ThmId => "parseDisplacement_32";

  /// `scaleReg (sc: nat) (ix: hex): nat`
  scaleReg: TermId => "scaleReg";

  /// `parseSI (sc: nat) (ix: hex) (osi: nat): wff`
  parseSI: TermId => "parseSI";
  parseSI_n[n: 16]: ThmId => format!("parseSI_{:x}", n);

  /// `sibSideCond (base: hex) (md: nat): wff`
  sibSideCond: TermId => "sibSideCond";
  sibSideCond_M[m: 3]: ThmId if m != 0 => format!("sibSideCond_M{}", m);
  sibSideCond_B[b: 16]: ThmId if b != 5 => format!("sibSideCond_B{:x}", b);

  /// `modrmSideCond (base md: nat): wff`
  modrmSideCond: TermId => "modrmSideCond";
  modrmSideCond_m[m: 3]: ThmId if m != 0 => format!("modrmSideCond_5{}", m);
  modrmSideCond_n[n: 8]: ThmId if n != 4 && n != 5 => format!("modrmSideCond_{:x}", n);

  /// `parseModRM2 (rex rm rm2 mod: nat) (l l2: string): wff`
  parseModRM2: TermId => "parseModRM2";
  parseModRM2_reg: ThmId => "parseModRM2_reg";
  parseModRM2_rip: ThmId => "parseModRM2_rip";
  parseModRM2_sib0: ThmId => "parseModRM2_sib0";
  parseModRM2_sibReg: ThmId => "parseModRM2_sibReg";
  parseModRM2_disp: ThmId => "parseModRM2_disp";

  /// `parseModRM (rex: nat) (rn: hex) (rm: nat) (l l2: string): wff`
  parseModRM: TermId => "parseModRM";
  parseModRM_S: ThmId => "parseModRM_S";
  parseModRM_1: ThmId => "parseModRM_1";

  /// `parseBinop (op: hex) (sz: nat) (dst: hex) (src: nat) (I: set): wff`
  parseBinop: TermId => "parseBinop";
  parseBinopBinop: ThmId => "parseBinopBinop";
  parseBinopClear32: ThmId => "parseBinopClear32";
  parseBinopClear64: ThmId => "parseBinopClear64";

  /// `hasREX (rex: nat) (b: wff): wff`
  hasREX: TermId => "hasREX";
  hasREX0: ThmId => "hasREX0";
  hasREXS: ThmId => "hasREXS";

  /// `instBinop (opc: hex) (sz: nat) (dst: hex) (src: nat): set`
  instBinop: TermId => "instBinop";
  /// `instShift (opc: hex) (sz: nat) (dst: hex) (src: nat): set`
  instShift: TermId => "instShift";
  /// `instImm (sz: nat) (dst: hex) (src: nat): set`
  instImm: TermId => "instImm";
  /// `instMovSX (dst_sz: nat) (dst: hex) (src_sz src: nat): set`
  instMovSX: TermId => "instMovSX";
  /// `instMovZX (dst_sz: nat) (dst: hex) (src_sz src: nat): set`
  instMovZX: TermId => "instMovZX";
  /// `instMov (sz dst src: nat): set`
  instMov: TermId => "instMov";
  /// `instPush (src: nat): set`
  instPush: TermId => "instPush";
  /// `instPop (dst: hex): set`
  instPop: TermId => "instPop";
  /// `instJump (tgt: nat): set`
  instJump: TermId => "instJump";
  /// `instJCC (c: hex) (tgt: nat): set`
  instJCC: TermId => "instJCC";
  /// `instCall (tgt: nat): set`
  instCall: TermId => "instCall";
  /// `instRet: set`
  instRet: TermId => "instRet";
  /// `instCDX (sz: nat): set`
  instCDX: TermId => "instCDX";
  /// `instLea (sz dst si base off: nat): set`
  instLea: TermId => "instLea";
  /// `instTest (sz src1 src2: nat): set`
  instTest: TermId => "instTest";
  /// `instUnop (op sz: nat) (dst: hex): set`
  instUnop: TermId => "instUnop";
  /// `instMul (sz src: nat): set`
  instMul: TermId => "instMul";
  /// `instDiv (sz src: nat): set`
  instDiv: TermId => "instDiv";
  /// `instSetCC (c: hex) (b: wff) (dst: hex): set`
  instSetCC: TermId => "instSetCC";
  /// `instCMov (c: hex) (sz dst src: nat): set`
  instCMov: TermId => "instCMov";
  /// `instSysCall: set`
  instSysCall: TermId => "instSysCall";
  /// `instUD2: set`
  instUD2: TermId => "instUD2";
  /// `instAssert (c: hex) (tgt: nat): set`
  instAssert: TermId => "instAssert";

  /// `parseOpc (p ip: nat) (s: string) (rex: nat) (opc: char) (I: set): wff`
  parseOpc: TermId => "parseOpc";
  parseFallthrough: ThmId => "parseFallthrough";
  parseBinopRAX: ThmId => "parseBinopRAX";
  parseBinopImm: ThmId => "parseBinopImm";
  parseBinopImm8: ThmId => "parseBinopImm8";
  parseBinopReg: ThmId => "parseBinopReg";
  parseBinopHi: ThmId => "parseBinopHi";
  parseBinopHi1: ThmId => "parseBinopHi1";
  parseBinopHiReg: ThmId => "parseBinopHiReg";
  parseMovSLQ: ThmId => "parseMovSLQ";
  parseMovSB: ThmId => "parseMovSB";
  parseMovZB: ThmId => "parseMovZB";
  parseMovStore: ThmId => "parseMovStore";
  parseMovLoad: ThmId => "parseMovLoad";
  parseMovZLQ: ThmId => "parseMovZLQ";
  parseMov32: ThmId => "parseMov32";
  parseMov64: ThmId => "parseMov64";
  parseMovImm: ThmId => "parseMovImm";
  parsePushImm8: ThmId => "parsePushImm8";
  parsePushImm32: ThmId => "parsePushImm32";
  parsePushReg: ThmId => "parsePushReg";
  parsePushMem: ThmId => "parsePushMem";
  parsePopReg: ThmId => "parsePopReg";
  parseJump8: ThmId => "parseJump8";
  parseJump32: ThmId => "parseJump32";
  parseJCC8: ThmId => "parseJCC8";
  parseJCCTwo: ThmId => "parseJCCTwo";
  parseCall: ThmId => "parseCall";
  parseRet: ThmId => "parseRet";
  parseCDQ: ThmId => "parseCDQ";
  parseCQO: ThmId => "parseCQO";
  parseLea32: ThmId => "parseLea32";
  parseLea64: ThmId => "parseLea64";
  parseTest: ThmId => "parseTest";
  parseTestRAX: ThmId => "parseTestRAX";
  parseTestHi: ThmId => "parseTestHi";
  parseInc: ThmId => "parseInc";
  parseDec: ThmId => "parseDec";
  parseNot: ThmId => "parseNot";
  parseNeg: ThmId => "parseNeg";
  parseMul: ThmId => "parseMul";
  parseDiv: ThmId => "parseDiv";
  parseSetCC: ThmId => "parseSetCC";
  parseCMov: ThmId => "parseCMov";
  parseSysCall: ThmId => "parseSysCall";
  parseUD2: ThmId => "parseUD2";
  parseAssert: ThmId => "parseAssert";

  // basicElf_ok: ThmId => "basicElf_ok";
}

pub(crate) enum Rex {
  B = 0,
  X = 1,
  R = 2,
  W = 3,
}

const SPLIT_BITS_PART: [&[u8]; 5] = [&[1, 3], &[2, 2], &[3, 1], &[1, 2, 1], &[1, 1, 1, 1]];
const SPLIT_BITS_NAMES: [&str; 5] = ["13", "22", "31", "121", "1111"];
#[derive(Copy, Clone)]
pub(crate) enum SplitBits {
  Sb13 = 0,
  Sb22 = 1,
  Sb31 = 2,
  Sb121 = 3,
  Sb1111 = 4,
}
impl SplitBits {
  pub(crate) const fn part(self) -> &'static [u8] { SPLIT_BITS_PART[self as usize] }
}
