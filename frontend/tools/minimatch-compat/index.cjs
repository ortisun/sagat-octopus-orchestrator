'use strict';

// Keep legacy callable imports and modern named imports on the same patched engine.
// No matching algorithm or removed v3/v5 RegExp API is reimplemented here.
const upstream = require('minimatch-upstream');
module.exports = upstream.minimatch;
module.exports.minimatch = upstream.minimatch;
module.exports.Minimatch = upstream.Minimatch;
module.exports.match = upstream.match;
module.exports.filter = upstream.filter;
module.exports.defaults = upstream.defaults;
module.exports.braceExpand = upstream.braceExpand;
module.exports.GLOBSTAR = upstream.GLOBSTAR;
module.exports.sep = upstream.sep;
module.exports.escape = upstream.escape;
module.exports.unescape = upstream.unescape;
